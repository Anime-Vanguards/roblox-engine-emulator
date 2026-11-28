--!nocheck
-- Rust engine: the service stubs of runtime/engine/services.luau that return script data (Players
-- lookups, 3D model helpers, humanoid stand-ins, EditableImage, HttpService
-- JSON, ContentProvider, marketplace / policy / social stubs, remotes). Kept in
-- Luau: they build script tables, may yield, and raise their errors at script level exactly as
-- the Luau runtime's do. Hot behaviours (DataModel, RunService, TweenService, ...) are Rust.
--
-- Runs with the shared script globals; R holds engine primitives (see lua/stubs.rs).

local R = ...
local Method = R.Method
local Getter = R.Getter
local ContentSetter = R.ContentSetter
local Create = R.Create
local RawSet = R.RawSet
local RawGet = R.RawGet
local Children = R.Children
local Missing = R.Missing
-- A method's string parameter as a client reads it (lua/convert.rs string_param, measured in a playing client and a server, Studio
-- 0.747: tools/parity/probes/string_parameter_methods.luau): a string as it is; nil raises "Argument n missing or nil"; a number
-- becomes its `%.20g` text, and any other value, after the engine warns `argument #n expects a string, but <type> was passed`
-- with the script's stack, the text the engine writes for it ("{}", "Instance", an EnumItem's name, ...)
local CastStringParam = R.StringParam
local WarnStringParam = R.WarnStringParam
local StringParam = {}
function StringParam.Read(Value, Argument)
	if type(Value) == "string" then
		return Value
	elseif Value == nil then
		error(`Argument {Argument} missing or nil`, 0)
	end
	return CastStringParam(Value, Argument)
end
local Options = R.Options
local Cost = R.Cost
local Mute = R.Mute
local Charge = R.Charge
local Pack, Unpack = table.pack, table.unpack; --[[ a CFrame parameter as a client casts it (PivotTo: measured, Studio 0.747, tools/parity/probes/instance_api_texts.luau): nil "Argument 1 missing or nil", another value "Unable to cast double to CoordinateFrame", named as camera.rs cast_name names it, without a position ]] local function CastCFrame(Value) if Value == nil then error("Argument 1 missing or nil", 0) end local Kind = typeof(Value) if Kind == "table" then Kind = if rawlen(Value) > 0 or next(Value) == nil then "Array" else "Dictionary" elseif Kind == "EnumItem" then Kind = tostring(Value.EnumType) else Kind = ({ number = "double", boolean = "bool", ["function"] = "Function", thread = "null", Enum = "null", Enums = "null", userdata = "null", RBXScriptSignal = "EventInstance" })[Kind] or Kind end error(`Unable to cast {Kind} to CoordinateFrame`, 0) end

-- Methods Studio does in one native call that this file does in Luau price themselves (cost.rs: the whole call, measured
-- in Studio); the Instance API's charges for the reads and writes made here meanwhile are dropped.
local function Priced(Body)
	return function(...)
		Mute(true)
		local Results = Pack(pcall(Body, ...))
		Mute(false)
		if not Results[1] then
			error(Results[2], 0)
		end
		return Unpack(Results, 2, Results.n)
	end
end

local Extras = setmetatable({}, { __mode = "k" })
local function Extra(Instance_)
	local Found = Extras[Instance_]
	if not Found then
		Found = {}
		Extras[Instance_] = Found
	end
	return Found
end

-- The error level of the script line that called a stub method: a stub runs under its method's C function
-- (lua/instance.rs lua_method), which `x:Method()` reaches through the Instance __namecall (lua/raw.rs), one
-- level further up than `x.Method(x)`. `Level` is relative to the stub, as if it were the method itself.
local function ScriptLevel(Level)
	local Source, Name = debug.info(Level + 2, "sn")
	if Source == "[C]" and Name == "__namecall" then
		return Level + 2
	end
	return Level + 1
end

local function EmptyPages()
	return {
		IsFinished = true,
		GetCurrentPage = function()
			return {}
		end,
		AdvanceToNextPageAsync = function() end,
	}
end

local function EnumItem(EnumName, ItemName)
	return R.GetEnumItem(EnumName, ItemName)
end

local function LocalPlayer()
	return R.LocalPlayer()
end

-- Players

Method("Players", "GetUserThumbnailAsync", function(_, UserId, ThumbnailType, ThumbnailSize)
	local TypeName = if ThumbnailType then ThumbnailType.Name else "HeadShot"
	local SizeName = if ThumbnailSize then ThumbnailSize.Name else "Size420x420"
	local Width, Height = string.match(SizeName, "(%d+)x(%d+)")
	local Kind = if TypeName == "AvatarBust" then "AvatarBust" elseif TypeName == "AvatarThumbnail" then "Avatar" else "AvatarHeadShot"
	return `rbxthumb://type={Kind}&id={UserId}&w={Width or 420}&h={Height or 420}`, true
end)

-- Measured in Studio (edit mode) for the user ids the project's stories use: the accounts' names,
-- and for ids without an account the error below; other ids fall back to User<id> offline.
local KnownUserNames = {
	[1] = "Roblox", [2] = "John Doe", [3] = "Jane Doe", [6] = "qa", [8] = "Toolbox", [33] = "morgan",
	[50] = "elvisarea51", [77] = "EZWrighter", [88] = "raabs", [99] = "OneRaven", [101] = "mraab",
	[102] = "sean", [103] = "opi511", [104] = "Thomas H", [105] = "Zap", [156] = "builderman",
	[200] = "pjjexpert", [261] = "Shedletsky",
}
local UnknownUserIds = { [4] = true, [5] = true, [7] = true, [9] = true, [10] = true, [11] = true, [12] = true }
Method("Players", "GetNameFromUserIdAsync", function(_, UserId)
	local Player = LocalPlayer()
	if Player and RawGet(Player, "UserId") == UserId then
		return RawGet(Player, "Name")
	end
	if KnownUserNames[UserId] then
		return KnownUserNames[UserId]
	end
	if UnknownUserIds[UserId] then
		error("Players:GetUserIdFromNameAsync() failed: Unknown user", 0)
	end
	Missing("Players", "GetNameFromUserIdAsync") return `User{UserId}` -- (an account's name is the web's: a stand-in)
end)
local function HumanoidDescription(Name) -- (an avatar is the web's: a default description stands in, and says so)
	return function() Missing("Players", Name) return Create("HumanoidDescription") end
end
Method("Players", "GetHumanoidDescriptionFromUserId", HumanoidDescription("GetHumanoidDescriptionFromUserId"))
Method("Players", "GetHumanoidDescriptionFromUserIdAsync", HumanoidDescription("GetHumanoidDescriptionFromUserIdAsync"))
Method("Players", "GetHumanoidDescriptionFromOutfitId", HumanoidDescription("GetHumanoidDescriptionFromOutfitId"))
Method("Players", "GetHumanoidDescriptionFromOutfitIdAsync", HumanoidDescription("GetHumanoidDescriptionFromOutfitIdAsync"))
Method("Players", "GetFriendsAsync", function()
	return EmptyPages()
end)
-- A bare R15-less rig: a Model holding a Humanoid (no meshes offline), a stand-in that reports itself.
local function HumanoidModel(Name)
	Missing("Players", Name) local Model = Create("Model")
	local Humanoid = Create("Humanoid")
	R.SetParent(Humanoid, Model)
	return Model
end
for _, Name in { "CreateHumanoidModelFromDescription", "CreateHumanoidModelFromDescriptionAsync", "CreateHumanoidModelFromUserId", "CreateHumanoidModelFromUserIdAsync" } do
	Method("Players", Name, function() return HumanoidModel(Name) end)
end

-- UserService / TextChatService (offline: other users exist and may chat)
local UserInfoIds = { [1] = true, [2] = true, [3] = true, [8] = true, [33] = true, [50] = true, [77] = true, [156] = true, [200] = true, [261] = true }
local VerifiedUserIds = { [1] = true, [156] = true, [261] = true }
local function UserInfo(UserId)
	local Player = LocalPlayer()
	if Player and RawGet(Player, "UserId") == UserId then
		return { Id = UserId, Username = RawGet(Player, "Name"), DisplayName = RawGet(Player, "DisplayName"), HasVerifiedBadge = false }
	end
	if UserInfoIds[UserId] then
		return { Id = UserId, Username = KnownUserNames[UserId], DisplayName = KnownUserNames[UserId], HasVerifiedBadge = VerifiedUserIds[UserId] == true }
	end
	if KnownUserNames[UserId] or UnknownUserIds[UserId] then
		return nil
	end
	return { Id = UserId, Username = `User{UserId}`, DisplayName = `User{UserId}`, HasVerifiedBadge = false }
end
Method("UserService", "GetUserInfosByUserIdsAsync", function(_, UserIds)
	local Result = {}
	for _, UserId in UserIds or {} do
		local Info = UserInfo(UserId)
		if Info then
			table.insert(Result, Info)
		end
	end
	return Result
end)
Method("TextChatService", "CanUserChatAsync", function()
	return true
end)
Method("TextChatService", "CanUsersChatAsync", function()
	return true
end)
Method("TextChatService", "CanUsersDirectChatAsync", function(_, _RequesterUserId, UserIds)
	return table.clone(UserIds or {})
end)

-- 3D geometry used by viewport framing and model presentation.

local function PartBounds(Root, Basis)
	Basis = Basis or CFrame.new()
	local Min, Max = nil, nil
	local function Visit(Node)
		if Node:IsA("BasePart") then
			local Transform = Basis:Inverse() * RawGet(Node, "CFrame")
			local Half = RawGet(Node, "Size") / 2
			for X = -1, 1, 2 do
				for Y = -1, 1, 2 do
					for Z = -1, 1, 2 do
						local Point = Transform * Vector3.new(Half.X * X, Half.Y * Y, Half.Z * Z)
						Min = if Min then Min:Min(Point) else Point
						Max = if Max then Max:Max(Point) else Point
					end
				end
			end
		end
		for _, Child in Children(Node) do
			Visit(Child)
		end
	end
	Visit(Root)
	if not Min then
		return nil, Vector3.zero
	end
	return Basis * CFrame.new((Min + Max) / 2), Max - Min
end

local function GetPivotInner(Self)
	if Self:IsA("BasePart") then
		return RawGet(Self, "CFrame") * RawGet(Self, "PivotOffset")
	end
	-- Measured (Studio 2026-10-16): a model's PrimaryPart, else its stored WorldPivotData (files,
	-- clones, PivotTo and WorldPivot writes set it), else the centre of its parts while it is unset.
	local Primary = if Self:IsA("Model") then RawGet(Self, "PrimaryPart") else nil
	if Primary then
		return GetPivotInner(Primary)
	end
	local Stored = if Self:IsA("Model") then RawGet(Self, "WorldPivotData", true) else Extra(Self).Pivot
	if Stored then
		return Stored
	end
	local Center = PartBounds(Self, R.PivotBasis(Self)) -- (the bounding box about the part covering the most ground: R.PivotBasis, at the end)
	return Center or CFrame.new()
end
local function GetBoundingBoxInner(Self)
	local Pivot = GetPivotInner(Self)
	local Center, Size = PartBounds(Self, Pivot) -- (about the pivot itself, its position too: measured to the bit, 0.747)
	return Center or Pivot, Size
end

Method("PVInstance", "GetPivot", Priced(function(Self)
	Charge(Cost.ModelQuery)
	return GetPivotInner(Self)
end))
-- A non-part PVInstance's stored pivot: a model's WorldPivotData, any other's the emulator's own.
local function StorePivot(Self, Pivot)
	if Self:IsA("Model") then
		RawSet(Self, "WorldPivotData", Pivot, true)
	else
		Extra(Self).Pivot = Pivot
	end
end
local function StoredPivot(Self)
	return if Self:IsA("Model") then RawGet(Self, "WorldPivotData", true) else Extra(Self).Pivot
end
-- Moves many parts as one step: the assemblies they belong to settle once it is done (world/joints.rs),
-- instead of each write carrying its whole assembly along.
local function Together(Body)
	R.BeginMoves()
	local Ok, Error = pcall(Body)
	R.EndMoves()
	if not Ok then
		error(Error, 0)
	end
end
-- Every part under Self moves by Delta, and every nested model's stored pivot with it (measured: a nested model's stored
-- pivot moves with its ancestor). The parts it moved, and how many other descendants it looked at.
local function MoveDescendants(Self, Delta, Target, Pivot) -- (a Model's PivotTo: each part to Target * Pivot:ToObjectSpace(it), measured to the bit)
	local Parts, Descendants = 0, Self:GetDescendants()
	for _, Descendant in Descendants do
		if Descendant:IsA("BasePart") then
			Parts += 1
			Descendant.CFrame = if Target then R.PivotPlace(Self, Descendant, Target, Pivot, RawGet(Descendant, "CFrame")) else Delta * Descendant.CFrame
		elseif Descendant:IsA("Model") then
			local Inner = StoredPivot(Descendant)
			if Inner then
				StorePivot(Descendant, if Target then R.PivotPlace(Self, Descendant, Target, Pivot, Inner) else Delta * Inner)
			end
		end
	end
	return Parts, #Descendants - Parts
end
-- Measured (roblox-sim, Studio 0.746, 2026-11-07): after PivotTo the pivot is the target exactly, every call. Composing
-- Delta = Target * Pivot:Inverse() onto the old one instead let its float error compound: the inverse is the transpose, so a
-- pivot scaled by s came back scaled by s squared, and a model pivoted every frame (an attack effect's spin) lost its
-- rotation in about 25 frames (its start part went to [0 0 0; 0 1 0; 0 0 0], the parts it emitted to NaN).
Method("PVInstance", "PivotTo", Priced(function(Self, Target) if typeof(Target) ~= "CFrame" then CastCFrame(Target) end
	if Self:IsA("BasePart") then
		-- Measured (Studio 2026-10-16): a part's PivotTo carries every part under it, through Folders, Attachments and
		-- Models, as one rigid move (an effect that moves its FX parts so: ree left a nested Slash 1,915 studs behind)
		local Delta = Target * GetPivotInner(Self):Inverse()
		local Below = 0
		Together(function()
			Self.CFrame = Target * RawGet(Self, "PivotOffset"):Inverse()
			Below = MoveDescendants(Self, Delta)
		end)
		Charge(Cost.PivotSingle + Below * if Self:IsDescendantOf(workspace) then Cost.PivotPart else Cost.PivotLoose)
	else
		local Pivot = GetPivotInner(Self) R.PivotBegin(Self, Pivot)
		local Stored = StoredPivot(Self)
		local Moved, Others = 0, 0
		Together(function()
			Moved, Others = MoveDescendants(Self, nil, Target, Pivot)
		end)
		Charge(Cost.PivotBase + Moved * (if Self:IsDescendantOf(workspace) then Cost.PivotPart else Cost.PivotLoose) + Others * Cost.PivotDescendant) R.PivotDone(Self, Target) -- (each descendant that is no part: cost.rs PIVOT_DESCENDANT)
		-- (a model whose pivot is its own stores the target; one whose PrimaryPart sets it carries its stored pivot along)
		if not (Self:IsA("Model") and RawGet(Self, "PrimaryPart")) then
			StorePivot(Self, Target)
		elseif Stored then
			StorePivot(Self, Target * Pivot:ToObjectSpace(Stored))
		end
	end
end))
Method("Model", "GetBoundingBox", Priced(function(Self)
	Charge(Cost.ModelQuery)
	return GetBoundingBoxInner(Self)
end))
Method("Model", "GetExtentsSize", Priced(function(Self)
	Charge(Cost.ModelExtentsQuery)
	local _, Size = GetBoundingBoxInner(Self)
	return Size
end))
-- The scale is a property, as in Roblox: files store it (serialized as ScaleFactor, read by rbx-dom as
-- Scale) and clones keep it, so a model saved at 0.01 reports 0.01 and ScaleTo(1) grows it 100x.
Method("Model", "GetScale", function(Self)
	Charge(Cost.ModelQuery)
	local Scale = RawGet(Self, "Scale")
	return if type(Scale) == "number" and Scale > 0 then Scale else 1
end)
-- ScaleTo scales every property with a geometric meaning (Model:ScaleTo's documentation: joint lengths,
-- particle sizes, sound roll-off; the Model Scale API announcement: "sizes, forces, speeds", Textures and
-- SurfaceGuis stretching like a Decal). Measured in Studio 2026-10-17 (ScaleTo 0.1, 1.6 and 0.5): every
-- property scaled below scales, and a Light's Range and a ParticleEmitter's or Beam's ZOffset keep theirs.
-- A Beam's or Trail's TextureLength is a
-- repeat count in Stretch mode and keeps it (devforum "ScaleTo incorrectly scales Trail.TextureLength",
-- April 2025: Roblox then scaled it in every mode, and the engineer who wrote ScaleTo answered that Stretch
-- should not scale and Wrap / Static should; Trail.WidthScale is a scale and keeps it too).
local function ScaledSequence(Sequence, Ratio)
	local Points = {}
	for _, Point in Sequence.Keypoints do
		table.insert(Points, NumberSequenceKeypoint.new(Point.Time, Point.Value * Ratio, Point.Envelope * Ratio))
	end
	return NumberSequence.new(Points)
end
local function ScaledUDim2(Size, Ratio)
	return UDim2.new(Size.X.Scale * Ratio, Size.X.Offset, Size.Y.Scale * Ratio, Size.Y.Offset)
end
local function ScaleEffect(Descendant, Ratio)
	if Descendant:IsA("ParticleEmitter") then
		Descendant.Size = ScaledSequence(Descendant.Size, Ratio)
		Descendant.Speed = NumberRange.new(Descendant.Speed.Min * Ratio, Descendant.Speed.Max * Ratio)
		Descendant.Acceleration *= Ratio
	elseif Descendant:IsA("Beam") then
		Descendant.Width0 *= Ratio
		Descendant.Width1 *= Ratio
		Descendant.CurveSize0 *= Ratio
		Descendant.CurveSize1 *= Ratio
		if Descendant.TextureMode ~= Enum.TextureMode.Stretch then
			Descendant.TextureLength *= Ratio
		end
	elseif Descendant:IsA("Trail") then
		Descendant.MinLength *= Ratio
		Descendant.MaxLength *= Ratio
		if Descendant.TextureMode ~= Enum.TextureMode.Stretch then
			Descendant.TextureLength *= Ratio
		end
	elseif Descendant:IsA("Fire") then
		Descendant.Size *= Ratio
		Descendant.Heat *= Ratio
	elseif Descendant:IsA("Smoke") then
		Descendant.Size *= Ratio
		Descendant.RiseVelocity *= Ratio
	elseif Descendant:IsA("Sound") then
		Descendant.RollOffMinDistance *= Ratio
		Descendant.RollOffMaxDistance *= Ratio
	elseif Descendant:IsA("Texture") then
		Descendant.StudsPerTileU *= Ratio
		Descendant.StudsPerTileV *= Ratio
		Descendant.OffsetStudsU *= Ratio
		Descendant.OffsetStudsV *= Ratio
	elseif Descendant:IsA("SurfaceGui") then
		if Descendant.SizingMode == Enum.SurfaceGuiSizingMode.PixelsPerStud then
			Descendant.PixelsPerStud /= Ratio
		end
	elseif Descendant:IsA("BillboardGui") then
		-- (a size's Scale is studs; its Offset pixels)
		Descendant.Size = ScaledUDim2(Descendant.Size, Ratio)
		Descendant.StudsOffset *= Ratio
		Descendant.StudsOffsetWorldSpace *= Ratio
	end
end
Method("Model", "ScaleTo", Priced(function(Self, Scale)
	assert(type(Scale) == "number" and Scale > 0 and Scale < math.huge, "Scale must be positive and finite")
	local Ratio = Scale / Self:GetScale()
	local Pivot = Self:GetPivot()
	local Looked, Parts, Attachments, Joints = 0, 0, 0, 0
	local function Scaled(Transform)
		return Transform.Rotation + Transform.Position * Ratio
	end
	Together(function()
		for _, Descendant in Self:GetDescendants() do
			Looked += 1
			ScaleEffect(Descendant, Ratio)
			if Descendant:IsA("BasePart") then
				Parts += 1
				Descendant.Size *= Ratio
				Descendant.CFrame = Pivot * Scaled(Pivot:ToObjectSpace(Descendant.CFrame))
				Descendant.PivotOffset = Scaled(Descendant.PivotOffset)
			elseif Descendant:IsA("Model") then
				-- (measured 2026-10-16: a nested model's stored pivot scales about the pivot like its parts)
				local Inner = StoredPivot(Descendant)
				if Inner then
					StorePivot(Descendant, Pivot * Scaled(Pivot:ToObjectSpace(Inner)))
				end
			elseif Descendant:IsA("Attachment") then
				Attachments += 1
				Descendant.CFrame = Scaled(Descendant.CFrame)
			elseif Descendant:IsA("JointInstance") then
				Joints += 1
				Descendant.C0 = Scaled(Descendant.C0)
				Descendant.C1 = Scaled(Descendant.C1)
			elseif Descendant:IsA("WeldConstraint") then
				-- its parts' relative CFrame, once it took one (State bit 2; world/joints.rs)
				if bit32.band(RawGet(Descendant, "State") or 0, 2) ~= 0 then
					RawSet(Descendant, "CFrame0", Scaled(RawGet(Descendant, "CFrame0")), true)
				end
			elseif Descendant:IsA("DataModelMesh") then
				-- Only a file mesh's Scale is in studs: the other shapes scale with their part's Size, so
				-- ScaleTo keeps it (measured in Studio 2026-10-11: Head and Sphere stay 1.25, FileMesh 2.5)
				if Descendant:IsA("SpecialMesh") and Descendant.MeshType == Enum.MeshType.FileMesh then
					Descendant.Scale *= Ratio
				end
				Descendant.Offset *= Ratio
			end
		end
	end)
	RawSet(Self, "Scale", Scale, true)
	-- (Studio looks at every descendant either way; a rescale costs more for the parts, attachments and joints it rewrites, a placed model's parts most)
	local Cost_ = Looked * Cost.ScaleScanned
	if Ratio ~= 1 then
		Cost_ += Parts * (if Self:IsDescendantOf(workspace) then Cost.ScalePartPlaced else Cost.ScalePartLoose) + Attachments * Cost.ScaleAttachment + Joints * Cost.ScaleJoint
	end
	Charge(Cost_)
end))
local PrimaryPartCFrame -- Model:GetPrimaryPartCFrame and SetPrimaryPartCFrame (defined at this file's end: it moves no line)
Method("Model", "GetPrimaryPartCFrame", function(Self)
	return PrimaryPartCFrame(Self)
end)
Method("Model", "SetPrimaryPartCFrame", function(Self, ...)
	PrimaryPartCFrame(Self, select("#", ...), ...) end)
Method("Model", "MoveTo", function(Self, Position)
	if Self:IsDescendantOf(game) then R.MoveModel(Self, Position) end -- (the pivot's position to Position, its rotation kept: R.MoveModel)
end)
Method("Model", "TranslateBy", function(Self, Offset)
	Self:PivotTo(Self:GetPivot() + Offset)
end)

-- Animator / AnimationTrack are Rust (src/animation)
Method("Humanoid", "SetStateEnabled", function(Self, State, Enabled)
	Extra(Self)[`_state:{tostring(State)}`] = Enabled
end)
Method("Humanoid", "GetStateEnabled", function(Self, State)
	return Extra(Self)[`_state:{tostring(State)}`] ~= false
end)
Method("Humanoid", "GetState", function() -- (ree has no Humanoid state machine and simulates no character: these report themselves)
	Missing("Humanoid", "GetState") return EnumItem("HumanoidStateType", "None")
end)
for _, Name in { "ChangeState", "MoveTo", "Move", "ApplyDescription", "ApplyDescriptionAsync", "ApplyDescriptionReset", "BuildRigFromAttachments" } do
	Method("Humanoid", Name, function() Missing("Humanoid", Name) end)
end
Method("Humanoid", "GetAccessories", function(Self)
	local Result = {}
	for _, Child in Children(Self.Parent or Self) do
		if Child:IsA("Accessory") then
			table.insert(Result, Child)
		end
	end
	return Result
end)
Method("Humanoid", "GetAppliedDescription", function()
	Missing("Humanoid", "GetAppliedDescription") return Create("HumanoidDescription")
end)

-- Path2D: its control points, their errors and the curve queries are the engine's (path2d.rs), where a host's
-- layout reads the points. (These lines keep the count of the stubs that stood here, so the lines below keep
-- their numbers in recorded traces.)
--
--
--
--
--
--
--
--
--
--
--
--
--
--
--
--
--
--
--
--
--

-- EditableImage: RGBA pixels (images/rs: an image loaded from an asset holds the asset's, decoded; a new
-- one starts transparent black). One never written keeps its asset's id, so a SurfaceAppearance built
-- from it draws the asset as Roblox shows it; once written, every SurfaceAppearance built from it is
-- pointed at a PNG snapshot of its pixels, one per resumption that wrote it, which the recorder plays
-- back as the map changing. The Draw methods draw nothing offline.
local SnapshotsDue = {}
local SnapshotScheduled = false

local function ImageSize(Image)
	local Size = RawGet(Image, "Size")
	return math.floor(Size.X), math.floor(Size.Y)
end

local function ImagePixels(Image)
	local Data = Extra(Image)
	if not Data.Pixels then
		local Width, Height = ImageSize(Image)
		Data.Pixels = buffer.create(Width * Height * 4)
	end
	return Data.Pixels
end

-- A snapshot of the image as it is now, as a map's content (nil when it could not be written): one
-- file until the image is written again, whatever shows it.
local function Snapshot(Image)
	local Data = Extra(Image)
	if Data.Snapshot then
		return Data.Snapshot
	end
	local Width, Height = ImageSize(Image)
	local Path, Problem = R.SaveImage(Width, Height, ImagePixels(Image))
	if not Path then
		warn(`EditableImage: its pixels could not be kept for the render ({Problem})`)
		return nil
	end
	Data.Snapshot = Content.fromUri(`file://{Path}`)
	return Data.Snapshot
end

local function TakeSnapshots()
	SnapshotScheduled = false
	local Due = SnapshotsDue
	SnapshotsDue = {}
	for Image in Due do
		local Users = Extra(Image).Users
		if not Users or next(Users) == nil then
			continue
		end
		local Map = Snapshot(Image)
		if Map then
			for Surface, Properties in Users do
				for Property in Properties do
					-- (a SurfaceAppearance's map, else the string id a Content view writes; an engine write: scripts get no
					-- Changed event, as a client's image writes give none, but the recorder sees it change)
					RawSet(Surface, Property, if Surface:IsA("SurfaceAppearance") then Map else Map.Uri, "unsignalled")
				end
			end
		end
	end
end

local function Written(Image)
	local Data = Extra(Image)
	Data.Source = nil
	Data.Snapshot = nil
	if Data.Users and next(Data.Users) ~= nil then
		SnapshotsDue[Image] = true
		if not SnapshotScheduled then
			SnapshotScheduled = true
			task.defer(TakeSnapshots)
		end
	end
end

-- The rows of a Position/Size rectangle inside the image: (x, y, width, height), clipped.
local function Rectangle(Image, Position, Size)
	local Width, Height = ImageSize(Image)
	local X = math.clamp(math.floor(if Position then Position.X else 0), 0, Width)
	local Y = math.clamp(math.floor(if Position then Position.Y else 0), 0, Height)
	local W = math.clamp(math.floor(if Size then Size.X else Width), 0, Width - X)
	local H = math.clamp(math.floor(if Size then Size.Y else Height), 0, Height - Y)
	return X, Y, W, H, Width
end

-- (a game script may not call DrawTriangle: measured in a 0.747 client; DrawImageProjected and SampleImageProjected check their projection's Direction there, and project nothing here)
Method("EditableImage", "DrawTriangle", function() error("The current thread cannot call 'DrawTriangle' (lacking capability RobloxScript)", 0) end)
for _, Name in { "DrawImageProjected", "SampleImageProjected" } do
	Method("EditableImage", Name, function(Self, _, Projection)
		if type(Projection) ~= "table" or Projection.Direction == nil then error("EditableImage::DrawImageProjected Param ProjectionParams, is missing Direction", 0) end
		Missing("EditableImage", Name)
	end)
end

-- A colour channel and an opacity (1 - transparency) as the draw methods take them, in 0-1 (measured in a 0.747 client: `x * 256` cut toward zero and kept under 256, a negative one wrapping as a byte: a transparency of 1.5 draws at half); an argument a client requires
local function Channel(Value) local Scaled = Value * 256 Scaled = if Scaled < 0 then math.ceil(Scaled) else math.floor(Scaled) return (math.min(Scaled, 255) % 256) / 255 end
local function Opacity(Transparency) return Channel(1 - (Transparency or 0)) end
local function Required(Value, Index) if Value == nil then error(`Argument {Index} missing or nil`, 0) end return Value end

-- Combines Source(x, y) (r, g, b, a in 0-1, or nil to leave the pixel) into the clipped rectangle, each pixel as a client combines it (images.rs combine)
local function Draw(Self, X0, Y0, W0, H0, CombineType, Source, Solid)
	local Width, Height = ImageSize(Self)
	local X1, Y1 = math.max(X0, 0), math.max(Y0, 0)
	local X2, Y2 = math.min(X0 + W0, Width), math.min(Y0 + H0, Height)
	if X2 <= X1 or Y2 <= Y1 then
		return
	end
	local Kind = if typeof(CombineType) == "EnumItem" then CombineType.Name else "BlendSourceOver"
	local Pixels = ImagePixels(Self)
	for Y = Y1, Y2 - 1 do
		for X = X1, X2 - 1 do
			local SR, SG, SB, SA = Source(X - X0, Y - Y0)
			if SR then
				R.Combine(Pixels, (Y * Width + X) * 4, Kind, SR, SG, SB, SA, Solid)
			end
		end
	end
	Written(Self)
end

-- Studio 0.747 lists seven more ImageCombineType items (Darken, Lighten, Screen, Overlay, SoftLight, HardLight, Difference) and gives each Draw method an alphaBlendType (ImageAlphaType) after its other arguments, and plays neither yet (measured there): such a combine type, as an item, its name or its value, raises "EditableImage:DrawImage:
-- the requested Enum.ImageCombineType is not yet supported", and after it an alpha type other than Default raises "EditableImage:DrawImage: Enum.ImageAlphaType other than Default is not yet supported". The image is left as it was.
local UnsupportedCombine = { Darken = true, Lighten = true, Screen = true, Overlay = true, SoftLight = true, HardLight = true, Difference = true }

local function ItemName(EnumType, Value)
	if typeof(Value) == "EnumItem" then return Value.Name elseif type(Value) == "string" then return Value end
	if type(Value) == "number" then local Item = EnumType:FromValue(Value) return Item and Item.Name end
	return nil
end

local function CheckBlend(Name, CombineType, AlphaType)
	local Combine = ItemName(Enum.ImageCombineType, CombineType)
	if Combine and UnsupportedCombine[Combine] then
		error(`EditableImage:{Name}: the requested Enum.ImageCombineType is not yet supported`, 0)
	end
	local Alpha = ItemName(Enum.ImageAlphaType, AlphaType)
	if Alpha == "LockCanvasAlpha" or Alpha == "LockCanvasColor" then
		error(`EditableImage:{Name}: Enum.ImageAlphaType other than Default is not yet supported`, 0)
	end
end

-- (measured in a 0.747 client: DrawRectangle, DrawCircle and DrawLine require their combine type, DrawImage its combine type and its image)
Method("EditableImage", "DrawRectangle", function(Self, Position, Size, Color, Transparency, CombineType, AlphaBlendType)
	Required(CombineType, 5)
	CheckBlend("DrawRectangle", CombineType, AlphaBlendType)
	local R_, G, B, A = Channel(Color.R), Channel(Color.G), Channel(Color.B), Opacity(Transparency)
	Draw(Self, math.floor(Position.X), math.floor(Position.Y), math.floor(Size.X), math.floor(Size.Y), CombineType, function()
		return R_, G, B, A
	end, true)
end)

Method("EditableImage", "DrawImage", function(Self, Position, Image, CombineType, AlphaBlendType)
	Required(CombineType, 3)
	if typeof(Image) ~= "Instance" or not Image:IsA("EditableImage") then error("EditableImage:DrawImage expects non nil srcImage", 0) end
	CheckBlend("DrawImage", CombineType, AlphaBlendType)
	local Width, Height = ImageSize(Image)
	local Source = ImagePixels(Image)
	Draw(Self, math.floor(Position.X), math.floor(Position.Y), Width, Height, CombineType, function(X, Y)
		local Offset = (Y * Width + X) * 4
		return buffer.readu8(Source, Offset) / 255, buffer.readu8(Source, Offset + 1) / 255,
			buffer.readu8(Source, Offset + 2) / 255, buffer.readu8(Source, Offset + 3) / 255
	end)
end)
-- Whether a Draw call anti-aliases its edges: its AntiAliasing argument, Enabled by default (Roblox's EditableImage reference gives DrawCircle and DrawLine `antiAliasing: AntiAliasing = Enabled`).
local function Smooth(AntiAliasing) return not (typeof(AntiAliasing) == "EnumItem" and AntiAliasing.Name == "Disabled") end

-- One pixel of a shape (DrawCircle, DrawLine) covered by `Cover` (0-1): its colour combined at that share (images.rs combine: the alpha scaled, Overwrite mixed with the pixel)
local function Plot(Self, X, Y, Kind, Colour, Cover)
	local Width, Height = ImageSize(Self)
	if Cover > 0 and X >= 0 and Y >= 0 and X < Width and Y < Height then
		R.Combine(ImagePixels(Self), (Y * Width + X) * 4, Kind, Colour[1], Colour[2], Colour[3], Colour[4], true, Cover)
	end
end
local function KindOf(CombineType) return if typeof(CombineType) == "EnumItem" then CombineType.Name else "BlendSourceOver" end

-- DrawCircle (measured in a 0.747 client, 8 circles: the centre the pixel it lies in, the radius rounded, under 1 "EditableImage:DrawCircle expects radius must be > 0"; a pixel at distance d from the centre pixel covered smoothstep(radius - d) clamped to 0-1, all of it
-- inside when not smoothed, d^2 at most radius^2)
Method("EditableImage", "DrawCircle", function(Self, Center, Radius, Color, Transparency, CombineType, AntiAliasing, AlphaBlendType)
	Required(CombineType, 5)
	Radius = math.floor(Radius + 0.5)
	if Radius <= 0 then error("EditableImage:DrawCircle expects radius must be > 0", 0) end
	CheckBlend("DrawCircle", CombineType, AlphaBlendType)
	local Colour, Kind, Soft = { Channel(Color.R), Channel(Color.G), Channel(Color.B), Opacity(Transparency) }, KindOf(CombineType), Smooth(AntiAliasing)
	local CX, CY = math.floor(Center.X), math.floor(Center.Y)
	for Y = CY - Radius, CY + Radius do
		for X = CX - Radius, CX + Radius do
			local D2 = (X - CX) ^ 2 + (Y - CY) ^ 2
			local T = math.clamp(Radius - math.sqrt(D2), 0, 1)
			Plot(Self, X, Y, Kind, Colour, if Soft then T * T * (3 - 2 * T) elseif D2 <= Radius * Radius then 1 else 0)
		end
	end
	Written(Self)
end)

-- DrawLine: from the pixel each end lies in (an end under 0 "EditableImage:DrawLine expects p1 to be greater than or equal to 0, 0"); smoothed, Xiaolin Wu's line (measured in a 0.747 client, 15 lines: each end's pixels at half, a pixel plotted twice combined twice); not smoothed, one pixel a step along its longer axis, the nearest
Method("EditableImage", "DrawLine", function(Self, P1, P2, Color, Transparency, CombineType, AntiAliasing, AlphaBlendType)
	Required(CombineType, 5)
	CheckBlend("DrawLine", CombineType, AlphaBlendType)
	for Index, Point in { P1, P2 } do if Point.X < 0 or Point.Y < 0 then error(`EditableImage:DrawLine expects p{Index} to be greater than or equal to 0, 0`, 0) end end
	local Colour, Kind = { Channel(Color.R), Channel(Color.G), Channel(Color.B), Opacity(Transparency) }, KindOf(CombineType)
	local X0, Y0, X1, Y1 = math.floor(P1.X), math.floor(P1.Y), math.floor(P2.X), math.floor(P2.Y)
	local Steep = math.abs(Y1 - Y0) > math.abs(X1 - X0)
	if Steep then X0, Y0, X1, Y1 = Y0, X0, Y1, X1 end
	if X0 > X1 then X0, X1, Y0, Y1 = X1, X0, Y1, Y0 end
	-- (in single precision, as a client steps it: a Vector3's component is a float32)
	local function F32(Value) return Vector3.new(Value, 0, 0).X end
	local Gradient = if X1 == X0 then 1 else F32((Y1 - Y0) / (X1 - X0))
	local function Put(X, Y, Cover) if Steep then Plot(Self, Y, X, Kind, Colour, Cover) else Plot(Self, X, Y, Kind, Colour, Cover) end end
	if not Smooth(AntiAliasing) then
		for X = X0, X1 do Put(X, math.floor(Y0 + Gradient * (X - X0) + 0.5), 1) end
		Written(Self)
		return
	end
	-- (ends on whole pixels: each end's pixel at half, the one beside it at nothing)
	Put(X0, Y0, 0.5)
	Put(X1, Y1, 0.5)
	local Y = F32(Y0 + Gradient)
	for X = X0 + 1, X1 - 1 do
		local Row = math.floor(Y)
		Put(X, Row, F32(1 - F32(Y - Row)))
		Put(X, Row + 1, F32(Y - Row))
		Y = F32(Y + Gradient)
	end
	Written(Self)
end)

-- DrawImageTransformed(position, scale, rotation, image, options): the image scaled, turned by `rotation` degrees about its PivotPoint (default its centre) and placed with that pivot at `position`, sampled
-- bilinearly (SamplingMode Pixelated: the nearest pixel), combined by options.CombineType (default AlphaBlend), as Roblox's EditableImage reference describes it (measured in a 0.747 client: the same pixels but for a few a level apart, the sample's float rounding; no image "EditableImage:DrawImageTransformed expects non-nil srcImage")
Method("EditableImage", "DrawImageTransformed", function(Self, Position, Scale, Rotation, Image, Options)
	Options = Options or {}
	if typeof(Image) ~= "Instance" or not Image:IsA("EditableImage") then error("EditableImage:DrawImageTransformed expects non-nil srcImage", 0) end
	CheckBlend("DrawImageTransformed", Options.CombineType, Options.AlphaBlendType)
	local Width, Height = ImageSize(Image)
	local Source = ImagePixels(Image)
	local Pivot = Options.PivotPoint or Vector2.new(Width / 2, Height / 2)
	local SX, SY = Scale.X, Scale.Y
	if SX == 0 or SY == 0 then
		return
	end
	local Angle = math.rad(Rotation or 0)
	local C, S = math.cos(Angle), math.sin(Angle)
	local Nearest = typeof(Options.SamplingMode) == "EnumItem" and Options.SamplingMode.Name == "Pixelated"
	-- the destination box of the transformed image's corners
	local MinX, MinY, MaxX, MaxY = math.huge, math.huge, -math.huge, -math.huge
	for _, Corner in { { 0, 0 }, { Width, 0 }, { 0, Height }, { Width, Height } } do
		local LX, LY = (Corner[1] - Pivot.X) * SX, (Corner[2] - Pivot.Y) * SY
		local X, Y = Position.X + LX * C - LY * S, Position.Y + LX * S + LY * C
		MinX, MinY, MaxX, MaxY = math.min(MinX, X), math.min(MinY, Y), math.max(MaxX, X), math.max(MaxY, Y)
	end
	local X0, Y0 = math.floor(MinX), math.floor(MinY)
	local function Texel(X, Y)
		X, Y = math.clamp(X, 0, Width - 1), math.clamp(Y, 0, Height - 1)
		local Offset = (Y * Width + X) * 4
		return buffer.readu8(Source, Offset) / 255, buffer.readu8(Source, Offset + 1) / 255,
			buffer.readu8(Source, Offset + 2) / 255, buffer.readu8(Source, Offset + 3) / 255
	end
	local Combine = if typeof(Options.CombineType) == "EnumItem" then Options.CombineType else Enum.ImageCombineType.AlphaBlend
	Draw(Self, X0, Y0, math.ceil(MaxX) - X0 + 1, math.ceil(MaxY) - Y0 + 1, Combine, function(X, Y)
		-- the destination pixel's centre back into the source image
		local DX, DY = X0 + X + 0.5 - Position.X, Y0 + Y + 0.5 - Position.Y
		local U = (DX * C + DY * S) / SX + Pivot.X
		local V = (-DX * S + DY * C) / SY + Pivot.Y
		if U < 0 or V < 0 or U >= Width or V >= Height then
			return nil
		end
		if Nearest then
			return Texel(math.floor(U), math.floor(V))
		end
		local FU, FV = U - 0.5, V - 0.5
		local IU, IV = math.floor(FU), math.floor(FV)
		local TU, TV = FU - IU, FV - IV
		local R1, G1, B1, A1 = Texel(IU, IV)
		local R2, G2, B2, A2 = Texel(IU + 1, IV)
		local R3, G3, B3, A3 = Texel(IU, IV + 1)
		local R4, G4, B4, A4 = Texel(IU + 1, IV + 1)
		-- (the sample a byte, rounded, before it is combined: measured)
		local function Mix(P, Q, Rr, Ss)
			return math.floor(((P * (1 - TU) + Q * TU) * (1 - TV) + (Rr * (1 - TU) + Ss * TU) * TV) * 255 + 0.5) / 255
		end
		return Mix(R1, R2, R3, R4), Mix(G1, G2, G3, G4), Mix(B1, B2, B3, B4), Mix(A1, A2, A3, A4)
	end)
end)

-- (measured in a 0.747 client: a destroyed image reads as one transparent pixel, its Size 1, 1, and draws go on)
Method("EditableImage", "Destroy", function(Self)
	local Data = Extra(Self)
	RawSet(Self, "Size", Vector2.new(1, 1), true)
	Data.Pixels = nil
	Data.Users = nil
end)
-- (measured: a rectangle past the image's edge "EditableImage:ReadPixelsBuffer expects position + size to be within EditableImage bounds", WritePixelsBuffer's too, and a buffer of another length than the rectangle's "EditableImage:WritePixelsBuffer expects buffer length to equal size.X * size.Y * 4 bytes")
local function Within(Self, Name, Position, Size)
	local Width, Height = ImageSize(Self)
	if Position.X < 0 or Position.Y < 0 or Position.X + Size.X > Width or Position.Y + Size.Y > Height then error(`EditableImage:{Name} expects position + size to be within EditableImage bounds`, 0) end
end
Method("EditableImage", "ReadPixelsBuffer", function(Self, Position, Size)
	Within(Self, "ReadPixelsBuffer", Position, Size)
	local X, Y, W, H, Stride = Rectangle(Self, Position, Size)
	local Out = buffer.create(W * H * 4)
	local Pixels = ImagePixels(Self)
	for Row = 0, H - 1 do
		buffer.copy(Out, Row * W * 4, Pixels, ((Y + Row) * Stride + X) * 4, W * 4)
	end
	return Out
end)
Method("EditableImage", "WritePixelsBuffer", function(Self, Position, Size, Source)
	if buffer.len(Source) ~= math.floor(Size.X) * math.floor(Size.Y) * 4 then error("EditableImage:WritePixelsBuffer expects buffer length to equal size.X * size.Y * 4 bytes", 0) end
	Within(Self, "WritePixelsBuffer", Position, Size)
	local X, Y, W, H, Stride = Rectangle(Self, Position, Size)
	local Pixels = ImagePixels(Self)
	local Rows = math.min(H, buffer.len(Source) // math.max(W * 4, 1))
	for Row = 0, Rows - 1 do
		buffer.copy(Pixels, ((Y + Row) * Stride + X) * 4, Source, Row * W * 4, W * 4)
	end
	Written(Self)
end)
-- (measured in a 0.747 client: a Size whole numbers, cut down; under 1 "AssetService:CreateEditableImage expects size.X and size.Y to be positive", over 1024 "... to be less than 1024"; 512 by 512 by default)
local function CreateEditableImage(_, Settings)
	local Image = Create("EditableImage")
	local Size = if type(Settings) == "table" and Settings.Size then Settings.Size else Vector2.new(512, 512)
	Size = Vector2.new(math.floor(Size.X), math.floor(Size.Y))
	if Size.X < 1 or Size.Y < 1 then error("AssetService:CreateEditableImage expects size.X and size.Y to be positive", 0) end
	if Size.X > 1024 or Size.Y > 1024 then error("AssetService:CreateEditableImage expects size.X and size.Y to be less than 1024", 0) end
	RawSet(Image, "Size", Size, true)
	return Image
end
Method("AssetService", "CreateEditableImage", CreateEditableImage)
Method("AssetService", "CreateEditableImageAsync", function(Self, Source, Settings)
	local Uri = if typeof(Source) == "Content" and Source.SourceType == Enum.ContentSourceType.Uri
		then Source.Uri
		elseif type(Source) == "number" then `rbxassetid://{Source}`
		elseif type(Source) == "string" then Source
		else nil
	local Width, Height, Pixels, Problem = nil, nil, nil, nil
	if Uri then
		Width, Height, Pixels, Problem = R.LoadImage(Uri)
		if not Pixels then
			warn(`EditableImage: {Problem}; it starts blank`)
		end
	end
	local Image = CreateEditableImage(Self, if Pixels then { Size = Vector2.new(Width, Height) } else Settings)
	local Data = Extra(Image)
	Data.Source = Uri
	Data.Pixels = Pixels
	return Image
end)

-- CreateSurfaceAppearanceAsync: each map an EditableImage's content (Roblox takes no other); an image
-- loaded from an asset and never written gives the map that asset, a written one its snapshot, and
-- the map follows its later writes.
local SURFACE_MAPS = {
	ColorMap = "ColorMapContent",
	NormalMap = "NormalMapContent",
	RoughnessMap = "RoughnessMapContent",
	MetalnessMap = "MetalnessMapContent",
	EmissiveMask = "EmissiveMaskContent",
}
Method("AssetService", "CreateSurfaceAppearanceAsync", function(_, Maps)
	if type(Maps) ~= "table" then
		error(`invalid argument #1 to 'CreateSurfaceAppearanceAsync' (table expected, got {typeof(Maps)})`, ScriptLevel(2))
	end
	local Surface = Create("SurfaceAppearance")
	for Key, Property in SURFACE_MAPS do
		local Map = Maps[Key]
		if Map == nil then
			continue
		end
		if typeof(Map) ~= "Content" or Map.SourceType ~= Enum.ContentSourceType.Object then
			error(`CreateSurfaceAppearanceAsync: {Key} must be an EditableImage's Content`, 0)
		end
		local Image = Map.Object
		local Data = Extra(Image)
		Data.Users = Data.Users or setmetatable({}, { __mode = "k" })
		Data.Users[Surface] = Data.Users[Surface] or {}
		Data.Users[Surface][Property] = true
		Extra(Surface).Shown = Extra(Surface).Shown or {}
		Extra(Surface).Shown[Property] = Image R.ShowsImage(Surface)
		local Drawn = if Data.Source then Content.fromUri(Data.Source) else Snapshot(Image)
		RawSet(Surface, Property, Drawn or Map, true)
	end
	return Surface
end)
-- (its maps read back the image's Content, as a client's do: measured, editable_image_content.luau)
for _, Property in SURFACE_MAPS do
	R.ContentGetter("SurfaceAppearance", Property, function(Self, Value)
		local Held = Extras[Self]
		local Image = Held and Held.Shown and Held.Shown[Property]
		return if Image then Content.fromObject(Image) else Value
	end)
end

-- CreateDataModelContentAsync: an EditableMesh's or EditableImage's Content made into one of its own, a copy of the object
-- as it is (measured in a 0.747 client, tools/parity/probes/data_model_content.luau): it yields, then answers
-- Enum.CreateContentResult.Success and an Opaque Content (no Uri and no Object; each call's is its own), which
-- CreateMeshPartAsync, CreateEditableMeshAsync and image properties take, and which later edits of the object (or its
-- Destroy) do not reach; any other Content is refused, and so is a mesh without faces. Offline nothing is uploaded, so the
-- other results (PermissionDenied, UploadFailed, ...) never come.
Method("AssetService", "CreateDataModelContentAsync", function(_, Source)
	local Object = if typeof(Source) == "Content" and Source.SourceType == Enum.ContentSourceType.Object then Source.Object else nil
	local Uri
	if Object and Object.ClassName == "EditableMesh" then
		local Problem
		Uri, Problem = R.MeshCopy(Object)
		if not Uri then
			error(Problem, 0)
		end
	elseif Object and Object.ClassName == "EditableImage" then
		local Data = Extra(Object)
		local Drawn = if Data.Source then Content.fromUri(Data.Source) else Snapshot(Object)
		Uri = if Drawn then Drawn.Uri else ""
	else
		error("The input content type is not supported by AssetService::CreateDataModelContentAsync yet!", 0)
	end
	task.wait()
	return Enum.CreateContentResult.Success, R.OpaqueContent(Uri)
end)

-- Any other Content property that takes an image shows an EditableImage too (Roblox's EditableImage reference: "EditableImage can be used in any Content property which takes an image, such as ImageLabel.ImageContent or MeshPart.TextureContent", by Content.fromObject): the string id the property's Content view
-- writes holds the image's asset while it is never written, else a snapshot of its pixels, and follows its later writes as a SurfaceAppearance's maps do. A script reads the property back as a client does (measured in a 0.747 client, 13 properties: tools/parity/probes/editable_image_content.luau): the image's Content (written or destroyed since), and "" for
-- the string id; assigning either property anything else drops the image.
local IMAGE_CONTENT = {
	ImageLabel = { ImageContent = "Image" },
	ImageButton = { ImageContent = "Image", HoverImageContent = "HoverImage", PressedImageContent = "PressedImage" },
	ImageHandleAdornment = { ImageContent = "Image" },
	Decal = { ColorMapContent = "Texture", TextureContent = "Texture" },
	MeshPart = { TextureContent = "TextureID" },
	FileMesh = { TextureContent = "TextureId" },
	ParticleEmitter = { TextureContent = "Texture" },
	Beam = { TextureContent = "Texture" },
	Trail = { TextureContent = "Texture" },
}
local function ShownImage(Self, Key) local Held = Extras[Self] return Held and Held.Shown and Held.Shown[Key] end
local function Unshow(Self, Sibling)
	local Previous = ShownImage(Self, Sibling)
	if Previous then
		local Users = Extra(Previous).Users
		if Users and Users[Self] then
			Users[Self][Sibling] = nil
		end
		Extra(Self).Shown[Sibling] = nil
	end
end
local Wrapped = {}
for ClassName, Properties in IMAGE_CONTENT do
	for Property, Sibling in Properties do
		ContentSetter(ClassName, Property, function(Self, Value)
			Unshow(Self, Sibling)
			if typeof(Value) ~= "Content" or Value.SourceType ~= Enum.ContentSourceType.Object then
				return Value
			end
			local Image = Value.Object
			if typeof(Image) ~= "Instance" or not Image:IsA("EditableImage") then
				return Value
			end
			local Data = Extra(Image)
			Data.Users = Data.Users or setmetatable({}, { __mode = "k" })
			Data.Users[Self] = Data.Users[Self] or {}
			Data.Users[Self][Sibling] = true
			Extra(Self).Shown = Extra(Self).Shown or {}
			Extra(Self).Shown[Sibling] = Image R.ShowsImage(Self)
			return (if Data.Source then Content.fromUri(Data.Source) else Snapshot(Image)) or Value
		end)
		R.ContentGetter(ClassName, Property, function(Self, Value)
			local Image = ShownImage(Self, Sibling)
			return if Image then Content.fromObject(Image) else Value
		end)
		if not Wrapped[ClassName .. "." .. Sibling] then
			Wrapped[ClassName .. "." .. Sibling] = true
			ContentSetter(ClassName, Sibling, function(Self, Value) Unshow(Self, Sibling) return Value end)
			R.ContentGetter(ClassName, Sibling, function(Self, Value) return if ShownImage(Self, Sibling) then "" else Value end)
		end
	end
end

-- Lighting: the sun and moon by ClockTime and GeographicLatitude (ror's fit to Studio's
-- GetSunDirection, within 1e-4: the sun circles the Z axis on a circle tilted latitude - 23.5
-- degrees; the moon mirrors it in X and Y)
local function SunDirection(Self)
	local HourAngle = ((RawGet(Self, "ClockTime") or 14) - 12) / 24 * math.pi * 2
	local Tilt = math.rad((RawGet(Self, "GeographicLatitude") or 0) - 23.5)
	return Vector3.new(-math.sin(HourAngle) * math.cos(Tilt), math.cos(HourAngle) * math.cos(Tilt), math.sin(Tilt))
end
Method("Lighting", "GetSunDirection", SunDirection)
Method("Lighting", "GetMoonDirection", function(Self)
	local Sun = SunDirection(Self)
	return Vector3.new(-Sun.X, -Sun.Y, Sun.Z)
end)
Method("Lighting", "GetMinutesAfterMidnight", function(Self)
	return (RawGet(Self, "ClockTime") or 14) * 60
end)
Method("Lighting", "SetMinutesAfterMidnight", function(Self, Minutes)
	Self.ClockTime = (Minutes / 60) % 24
end)

-- Player
Method("Player", "GetMouse", function()
	-- the PlayerMouse, made on the first call (world/mouse.rs)
	local Mouse = R.Mouse
	if Mouse == nil then
		Mouse = R.PlayerMouse()
		R.Mouse = Mouse
	end
	return Mouse
end)
-- Groups: the offline player is in none (false, rank 0, role Guest), the Async methods as the older ones; the six share the ids a player was asked about, and the first call naming a group yields (a client asks the web), later ones answer at once (measured in a 0.747 client and server: parity loopback/player_groups)
for Member, Answer in { IsInGroup = false, IsInGroupAsync = false, GetRankInGroup = 0, GetRankInGroupAsync = 0, GetRoleInGroup = "Guest", GetRoleInGroupAsync = "Guest" } do
	Method("Player", Member, function(Self, GroupId)
		local Id, Message = R.Int64Argument(GroupId, 1)
		if typeof(Self) ~= "Instance" or not Self:IsA("Player") then error(`Expected ':' not '.' calling member function {Member}`, 0) elseif not Id then error(Message, 0) end
		local Data = Extra(Self) Data.Groups = Data.Groups or {}
		if not Data.Groups[Id] then task.wait() Data.Groups[Id] = true end return Answer
	end)
end
-- Friends: IsFriendsWith, GetFriendsOnline, their Async names, GetFriendsInServerAsync and GetFriendsInUniverseAsync are at
-- this file's end, where they move no line (warning traces name this file's lines). The offline player has no friends:
-- false and empty lists, after the checks and the wait a client's question of the web makes
Method("Player", "HasAppearanceLoaded", function()
	return true
end)
-- Character control without a character (Roblox's ControlModule binds Move at load). Move walks the player's character (Roblox's Player documentation), so without one it does nothing, as in a client; RequestStreamAroundAsync asks for what a run holds whole already (no instance streaming); the rest are not emulated
for _, Name in { "Kick", "Move", "LoadCharacter", "LoadCharacterAsync", "RequestStreamAroundAsync" } do
	Method("Player", Name, function(Self) if Name == "RequestStreamAroundAsync" or (Name == "Move" and RawGet(Self, "Character") == nil) then return end Missing("Player", Name) end)
end
Method("Player", "DistanceFromCharacter", function()
	return 0
end)
Method("Player", "GetJoinData", function()
	return {}
end)
-- (GetFriendsOnline and GetFriendsOnlineAsync are with the friends methods at this file's end: the local player's online
-- friends, after the count's cast, the warning past 200 friends, the local player check and the wait for the web's answer;
-- the lines here keep their place)
Method("Player", "IsVerified", function()
	return false
end)
Method("Player", "GetNetworkPing", function()
	return 0.05
end)

-- (HttpService JSON and requests are Rust: services/mod.rs)

local ContentIdArg -- ContentProvider (ContentIdArg, a method's ContentId argument, is defined at this file's end: it moves no line)
-- (it waits for the animation assets it asks for: animation/mod.rs preload. Its callback gets each Animation's id and
-- whether it loads, for an item that is or holds Animations, measured; any other item, whose content is not looked at,
-- its text and Success)
Method("ContentProvider", "PreloadAsync", function(_, Items, Callback)
	local Wait, Animations = R.PreloadAnimations(Items)
	if Wait > 0 then
		task.wait(Wait)
	elseif coroutine.isyieldable() and (function() for _, Item in Items do if type(Item) == "string" then return true end end return false end)() then
		task.wait() -- (a content id is fetched: a frame here, not measured. With nothing to load, an empty list or instances, a 0.747 client and
	end -- server return at once, where the thread can yield or not: no frame passes, a task.defer before has not run; yield_refused.luau)
	if Callback then
		for Index, Item in Items do
			local Held = Animations[Index]
			if Held then
				for _, Entry in Held do
					Callback(Entry[1], EnumItem("AssetFetchStatus", if Entry[2] then "Success" else "Failure"))
				end
			else
				local Id = if type(Item) == "string" then Item else tostring(Item)
				Callback(Id, EnumItem("AssetFetchStatus", "Success"))
			end
		end
	end
end)
Getter("ContentProvider", "RequestQueueSize", function()
	return 0
end)
Method("ContentProvider", "GetAssetFetchStatus", function(_, ContentId) -- (ree fetches no asset: every id reads None, as one a client never fetched does)
	return ContentIdArg(ContentId) and EnumItem("AssetFetchStatus", "None")
end)

-- KeyframeSequenceProvider:GetKeyframeSequenceAsync: after a yield, a new copy of the KeyframeSequence
-- the id names, found as AnimationTracks find theirs (animation/source.rs). An id that does not load
-- raises after the yield, without a position (the message is not measured). (RegisterKeyframeSequence
-- is Rust: animation/mod.rs)
Method("KeyframeSequenceProvider", "GetKeyframeSequenceAsync", function(_, AssetId)
	if type(AssetId) ~= "string" then
		ContentIdArg(AssetId)
	end
	task.wait()
	local Sequence, Message = R.KeyframeSequence(tostring(AssetId))
	if not Sequence then
		error(Message, 0)
	end
	return Sequence
end)

-- AnimationClipProvider:GetAnimationClipAsync: the same for the clip as it is, a KeyframeSequence or a
-- CurveAnimation (measured in Studio 0.747: an id that is no string raises "Unable to cast double to
-- ContentId"; the message of an id that does not load is this engine's own).
Method("AnimationClipProvider", "GetAnimationClipAsync", function(_, AssetId)
	if AssetId == nil then
		error("Argument 1 missing or nil", 0)
	end
	if type(AssetId) ~= "string" then
		ContentIdArg(AssetId)
	end
	task.wait()
	local Clip, Message = R.AnimationClip(AssetId)
	if Clip == false then
		-- (measured: the id of a registered instance that is no clip never answers)
		coroutine.yield()
	end
	if not Clip then
		error(Message, 0)
	end
	return Clip
end)

-- TextService: family metadata of a font family asset / json path
Method("TextService", "GetFamilyInfoAsync", function(_, AssetId)
	local Id = ContentIdArg(AssetId)
	return {
		Name = string.match(Id, "([^/]+)%.json$") or Id,
		Faces = {
			{ Name = "Regular", Weight = EnumItem("FontWeight", "Regular"), Style = EnumItem("FontStyle", "Normal") },
		},
	}
end)

-- SharedTableRegistry: a name holds one SharedTable, made on the first Get (measured in Studio 2026-10-23: a
-- fresh name returns a SharedTable, the same one on every call; SetSharedTable registers a given one, and nil
-- removes the name). SetSharedTable takes nothing else (Studio 0.748: "Unable to cast double to SharedTable", a
-- table named an Array or a Dictionary). SharedTables are userdata (lua/shared_table.rs), and each Get returns a new
-- handle of the name's table, == to the others but not rawequal (Studio 0.748, a playing client).
--
-- A name is cast to a string as a client casts it (Studio 0.748, a playing client; StringParam): a string as it is (a NUL and 70,000
-- bytes kept), a number as `%.20g` writes it (0.1 is "0.10000000000000000555", 2^63 "9223372036854775808", -0 "-0",
-- 1e-310 "9.9999999999999694493e-311"), NaN "NAN" and the infinities "INF" and "-INF". Anything else warns with the
-- script's stack ("argument #1 expects a string, but boolean was passed") and is then named as the engine writes that type:
-- a table "{}", an Instance "Instance", a function "function", a thread, an Enum or a newproxy "nil", an EnumItem its Name, a
-- Vector3 with nine digits a component ("0.100000001, 0.333333343, 2" where tostring gives "0.10000000149011612, ...";
-- "NAN, INF, -INF"), the rest as tostring writes it (a boolean, a CFrame, a Color3, a UDim2 and every other datatype tried).
-- The number and the Vector3 are written in Rust (lua/convert.rs cast_text), where a subnormal keeps its digits in the
-- scripts' float mode. A buffer is named by its contents and every SharedTable by one name, neither of which any text tried
-- reached ("abc", "", "{}", tostring, JSONEncode's, base64, hex, ...): here keys no string can be. nil raises "Argument 1
-- missing or nil", and SetSharedTable checks its value first (SetSharedTable(nil, 5) raises the cast).
local SharedTableName = newproxy()
local BufferNames = setmetatable({}, { __mode = "v" }) -- (a content's name lives while SharedTables holds it)
local BufferRegistryName = R.BufferRegistryName -- (the contents as a string, charged as one the script built: shared_table.rs)
local function RegistryName(Name)
	local Type = typeof(Name)
	if Type ~= "SharedTable" and Type ~= "buffer" then
		return StringParam.Read(Name, 1)
	end
	WarnStringParam(Name, 1)
	if Type == "SharedTable" then
		return SharedTableName
	else
		local Contents = BufferRegistryName(Name)
		local Found = BufferNames[Contents]
		if Found == nil then
			Found = newproxy()
			BufferNames[Contents] = Found
		end
		return Found
	end
end

-- What SetSharedTable's cast error calls a value that is not a SharedTable (measured): a number "double", a boolean "bool",
-- a table "Array" (empty or with an array part) or "Dictionary", a function "Function", a CFrame "CoordinateFrame", an
-- EnumItem its enum's name ("Material"), a thread, the Enum global or a newproxy "null" (an enum, as the global: not
-- measured), anything else its type.
local RegistryCastNames = { number = "double", boolean = "bool", ["function"] = "Function", CFrame = "CoordinateFrame", thread = "null", Enums = "null", Enum = "null", userdata = "null" }

-- A table the registry holds is marked, as a client marks it (lua/shared_table.rs `Shared::mark`, measured: the table Set
-- gets, also when its name then raises, and the one a Get makes).
local SharedTables = {}
local SharedTableHandle = R.SharedTableHandle
local MarkSharedTable = R.MarkSharedTable
Method("SharedTableRegistry", "GetSharedTable", function(_, Name)
	Name = RegistryName(Name)
	local Found = SharedTables[Name]
	if Found == nil then
		Found = SharedTable.new()
		MarkSharedTable(Found)
		SharedTables[Name] = Found
	end
	return SharedTableHandle(Found)
end)
Method("SharedTableRegistry", "SetSharedTable", function(_, Name, Value)
	if Value ~= nil and typeof(Value) ~= "SharedTable" then
		-- (a table's raw length, as SetAttribute names one: a client runs no __len here, measured)
		local Kind = if type(Value) == "table" then (if rawlen(Value) > 0 or next(Value) == nil then "Array" else "Dictionary") else typeof(Value)
		Kind = if Kind == "EnumItem" then tostring(Value.EnumType) else RegistryCastNames[Kind] or Kind
		error(`Unable to cast {Kind} to SharedTable`, 0)
	end
	MarkSharedTable(Value)
	SharedTables[RegistryName(Name)] = Value
end)

-- (CollectionService:GetAllTags is Rust: services/mod.rs)

-- GuiService.TopbarInset: the top bar's band right of Roblox's buttons (R.TopbarStart: services/mod.rs TopBar)
Getter("GuiService", "TopbarInset", function()
	if Options.TopbarInset > 0 then
		return Rect.new(R.TopbarStart(), 0, Options.ViewportWidth, Options.TopbarInset)
	end
	return Rect.new(0, 0, 0, 0)
end)

local InsetArea -- GuiService:GetInsetArea(insets): the area a ScreenGui of those insets lays out in, as a Rect; InsetArea, at the
-- end of this file (it casts the argument with CastEnum, defined below), is the measured rule (a panel's backdrop sizes itself
-- from it).
Method("GuiService", "GetInsetArea", function(_, Insets)
	return InsetArea(Insets)
	-- (the method keeps its place: the lines below it are named by warnings' traces)
	-- (see InsetArea)
end)

-- Marketplace / Policy / social stubs (LocalizationService, LocalizationTable and Translator are services/localization.rs)
local function GetProductInfo(_, AssetId, InfoType)
	Missing("MarketplaceService", "GetProductInfo (mocked)")
	return {
		Name = `Product {AssetId}`,
		Description = "",
		PriceInRobux = 0,
		IsForSale = true,
		IsPublicDomain = false,
		IsLimited = false,
		IsLimitedUnique = false,
		IconImageAssetId = 0,
		AssetId = AssetId,
		ProductId = AssetId,
		Creator = { Name = "Roblox", CreatorType = "Group", Id = 1 },
		ProductType = if InfoType then InfoType.Name else "Asset",
		Remaining = 0,
		Sales = 0,
	}
end
Method("MarketplaceService", "GetProductInfo", GetProductInfo)
Method("MarketplaceService", "GetProductInfoAsync", GetProductInfo)
for _, Name in { "UserOwnsGamePassAsync", "PlayerOwnsAsset", "PlayerOwnsBundle" } do
	Method("MarketplaceService", Name, function()
		return false
	end)
end
Method("MarketplaceService", "GetDeveloperProductsAsync", function()
	Missing("MarketplaceService", "GetDeveloperProductsAsync") return EmptyPages()
end)
for _, Name in { "PromptPurchase", "PromptProductPurchase", "PromptGamePassPurchase", "PromptPremiumPurchase", "PromptBundlePurchase" } do -- (PromptSubscriptionPurchase: at the end)
	Method("MarketplaceService", Name, function() Missing("MarketplaceService", Name) end)
end

Method("PolicyService", "GetPolicyInfoForPlayerAsync", function() -- (a playing 0.747 client's whole table offline, measured 2026-11-26; an offline server raised "Http request failed to get policy info.")
	return {
		AllowedExternalLinkReferences = {},
		AreAdsAllowed = true, ArePaidRandomItemsRestricted = false,
		IsContentSharingAllowed = true, IsEligibleToPurchaseCommerceProduct = false,
		IsEligibleToPurchaseSubscription = true, IsEndlessContentAutoplayAllowed = true,
		IsEndlessContentLoadAllowed = true, IsPaidItemTradingAllowed = true,
		IsPhotoToAvatarAllowed = false, IsSubjectToChinaPolicies = false,
		IsThirdPartyAccountLinkingAllowed = true,
	}
end)
Method("BadgeService", "UserHasBadgeAsync", function()
	return false
end)
Method("BadgeService", "GetBadgeInfoAsync", function()
	Missing("BadgeService", "GetBadgeInfoAsync") return { Name = "Badge", Description = "", IconImageId = 0, IsEnabled = true }
end)
Method("GroupService", "GetGroupInfoAsync", function(_, GroupId)
	Missing("GroupService", "GetGroupInfoAsync") return { Name = `Group {GroupId}`, Id = GroupId, Owner = { Name = "Owner", Id = 1 }, EmblemUrl = "", Roles = {} }
end)
Method("GroupService", "GetGroupsAsync", function()
	return {}
end)
-- Studio's answers for an offline player (an empty test place, 2026-10-23): no upcoming events, and a player who is in no group
Method("GroupService", "GetRolesInGroupAsync", function()
	return { IsMember = false, Roles = {} }
end)
Method("SocialService", "GetUpcomingExperienceEventsAsync", function()
	return {}
end)
Method("SocialService", "CanSendGameInviteAsync", function()
	return false
end)
Method("SocialService", "CanSendCallInviteAsync", function()
	return false
end)
Getter("VRService", "VREnabled", function()
	return false
end)
Method("TeleportService", "GetLocalPlayerTeleportData", function()
	return nil
end)
-- (a setting's name read as a client reads a string parameter, measured: StringParam)
Method("TeleportService", "GetTeleportSetting", function(_, Setting)
	StringParam.Read(Setting, 1)
	return nil
end)
Method("TeleportService", "SetTeleportSetting", function(_, Setting)
	StringParam.Read(Setting, 1)
	Missing("TeleportService", "SetTeleportSetting")
end) -- (not kept: GetTeleportSetting stays nil)
-- UserGameSettings (UserSettings().GameSettings): a client whose settings are as they start
for _, Name in { "GetOnboardingCompleted", "InFullScreen", "InStudioMode" } do
	Method("UserGameSettings", Name, function()
		return false
	end)
end
-- the camera's vertical multiplier: 1, or -1 when the player inverts it (measured in Studio 0.748: 1 by default)
Method("UserGameSettings", "GetCameraYInvertValue", function()
	return 1
end)
for _, Name in { "SetCameraYInvertVisible", "SetGamepadCameraSensitivityVisible", "SetOnboardingCompleted" } do
	Method("UserGameSettings", Name, function() if Name == "SetOnboardingCompleted" then Missing("UserGameSettings", Name) end end) -- (the other two only show options in Roblox's own settings menu, which no script reads)
end
-- AnalyticsService's Log methods: defined at this file's end, with their argument casts (the events go to the creator's analytics
-- dashboard; nothing a script can read changes but their warning: Roblox's AnalyticsService documentation, measured in a 0.747
-- playing client and a server)
-- (SoundService:PlayLocalSound is Rust: engine::world reports the play)

-- Remotes never reach a server offline; invocations yield forever like a dropped connection.
Method("RemoteFunction", "InvokeServer", function(Self, ...)
	R.Sendable(select("#", ...) + 1, ...) if not coroutine.isyieldable() then error("thread is not yieldable", 0) end if Self.Parent then Missing("RemoteFunction", `InvokeServer ({Self:GetFullName()}) never returns offline`) end -- (the arguments, then the refusal: tools/parity/probes/remote_call_order.luau)
	return coroutine.yield()
end)
Method("BindableFunction", "Invoke", function(Self, ...)
	local Callback = R.GetCallback(Self, "OnInvoke")
	if not Callback or not coroutine.isyieldable() then -- (where the thread cannot yield: R.InvokeApart, at the end)
		if Callback then return R.InvokeApart(Self, Callback, ...) elseif coroutine.isyieldable() then return coroutine.yield() end error("thread is not yieldable", 0)
	end
	if Self:GetActor() then
		Charge(Cost.ActorInvoke) -- (a call into an Actor's VM: 1.0 us in Studio)
	end
	-- (copied as Roblox copies them: lua/bindable.rs; the callback on a thread of its own: R.InvokeOwn, at the end)
	return R.InvokeCopied(R.InvokeOwn(Self, Callback, R.CopiedArguments(...)))
end)

-- LogService:Output, Info, Warn, Error and Log: a line of their MessageType (Output: print's, Info: MessageInfo, Warn: warn's,
-- Error: an error line, then a thrown error table), its `{key}` placeholders filled from the context. Measured in a playing
-- client (Studio 0.744; a server's game script took the same lines and order) beside Roblox's LogService documentation.
-- A placeholder runs from `{` to the next `}` and takes the context's value at that text (the table's own lookup: a
-- metatable's __index answers), written as tostring writes it (an Instance its Name); a key the context lacks keeps its
-- placeholder; `{{` and `}}` write one brace, and a lone `}` stays. Without a context table (none, or a number, a string, an
-- Instance) the message is left as it is, braces included.
local function LogRender(Template, Context)
	local Out = {}
	local Index, Length = 1, #Template
	while Index <= Length do
		local Next = string.find(Template, "[{}]", Index)
		if not Next then
			table.insert(Out, string.sub(Template, Index))
			break
		end
		table.insert(Out, string.sub(Template, Index, Next - 1))
		local Brace = string.sub(Template, Next, Next)
		if string.sub(Template, Next + 1, Next + 1) == Brace then
			table.insert(Out, Brace)
			Index = Next + 2
		elseif Brace == "}" then
			table.insert(Out, "}")
			Index = Next + 1
		else
			local Close = string.find(Template, "}", Next + 1, true)
			if not Close then
				table.insert(Out, string.sub(Template, Next))
				break
			end
			local Value = Context[string.sub(Template, Next + 1, Close - 1)]
			table.insert(Out, if Value == nil then string.sub(Template, Next, Close) else tostring(Value))
			Index = Close + 1
		end
	end
	return table.concat(Out)
end

-- The context as MessageOut and a thrown error carry it (measured): a new table each time, the given table's own keys (not a
-- metatable's) with their values copied. A context whose keys are all strings keeps its values' types: a function becomes
-- "<function>", an Instance its full name, a table a copy of its own (its keys as they are), and one that holds itself its
-- tostring text; a thread, an Enum and a plain userdata are left out, other values (numbers, strings, datatypes, EnumItems,
-- buffers) kept. A context with any other key (an array part, a number key) keeps strings, numbers and booleans and writes
-- every other value as tostring does (an Instance its Name, a Vector3 "1, 2, 3", a table "table: 0x..."). A number key becomes
-- its whole part's text (2.7 "2"; past int32's range "-2147483648"), a string key of the same text wins over it, other keys are
-- left out.
local function LogContextValue(Value, Path)
	local Kind = typeof(Value)
	if Kind == "function" then
		return "<function>"
	elseif Kind == "Instance" then
		return Value:GetFullName()
	elseif Kind == "table" then
		if Path[Value] then
			return nil, true
		end
		Path[Value] = true
		local Copy = {}
		for Key, Item in next, Value do
			local Converted, Cycle = LogContextValue(Item, Path)
			if Cycle then
				return nil, true
			end
			if Converted ~= nil then
				Copy[Key] = Converted
			end
		end
		Path[Value] = nil
		return Copy
	elseif Kind == "thread" or Kind == "Enum" or Kind == "userdata" then
		return nil
	end
	return Value
end
local function LogContext(Given)
	local Mixed = false
	for Key in next, Given do
		if type(Key) ~= "string" then
			Mixed = true
			break
		end
	end
	local Out, Numbered = {}, {}
	for Key, Value in next, Given do
		local Kind = type(Key)
		if Kind == "string" or Kind == "number" then
			local Converted, Cycle
			if Mixed then
				local ValueKind = type(Value)
				Converted = if ValueKind == "string" or ValueKind == "number" or ValueKind == "boolean" then Value else tostring(Value)
			else
				Converted, Cycle = LogContextValue(Value, {})
				if Cycle then
					Converted = tostring(Value)
				end
			end
			if Converted ~= nil then
				if Kind == "string" then
					Out[Key] = Converted
				else
					local Text = if Key == Key and Key > -2147483649 and Key < 2147483648
						then string.format("%d", if Key < 0 then math.ceil(Key) else math.floor(Key))
						else "-2147483648"
					Numbered[Text] = Converted
				end
			end
		end
	end
	for Text, Value in Numbered do
		if Out[Text] == nil then
			Out[Text] = Value
		end
	end
	return Out
end

-- The message as text, read as any string parameter is (StringParam: measured, Studio 0.748, a playing client: 0.1 is
-- "0.10000000000000000555", 1e-310 "9.9999999999999694493e-311"; a boolean, a table, an Instance, a function, an Enum and a
-- Vector3 warn with the script's stack, "argument #1 expects a string, but boolean was passed", and are written "true", "{}",
-- "Instance", "function", "nil" and "0.100000001, 0.333333343, 2")
local function LogMessage(Message, Argument)
	return StringParam.Read(Message, Argument)
end

local function LogSelf(Self, Member)
	if typeof(Self) ~= "Instance" or not Self:IsA("LogService") then
		error(`Expected ':' not '.' calling member function {Member}`, 0)
	end
end

Method("LogService", "GetLogHistory", function(Self)
	LogSelf(Self, "GetLogHistory")
	return R.LogHistory()
end)

-- The thrown error's tostring (measured): its message when that is a string or a number, else "Error occurred"
local function LogErrorText(Self)
	local Message = rawget(Self, "message")
	local Text = if type(Message) == "string" then Message elseif type(Message) == "number" then tostring(Message) else "Error occurred"
	R.LogErrorText(Text)
	return Text
end

-- One line: the message checked (`Position`: its argument's place counting the service, 2 or 3), written and its placeholders
-- filled; the line logged with a copy of the context. Returns the text and the template.
local function LogLine(Kind, Message, Given, Position)
	if Message == nil then
		error(`Argument {Position} missing or nil`, 0)
	end
	local Template = LogMessage(Message, Position - 1)
	local Context = if type(Given) == "table" then Given else nil
	local Text = if Context then LogRender(Template, Context) else Template
	R.LogLine(Kind, Text, if Context then LogContext(Context) else nil)
	Charge(Cost.LogCall)
	return Text, Template, Context
end

-- An error line, then the error table, thrown (measured): `message` (the text), `template` (the message as given), `context`
-- (a copy of its own; an empty table without one), `stack` ("Stack Begin", a `Script '<chunk>', Line <n>` line per frame of the
-- calling script, "Stack End"), and a metatable of its own whose __tostring gives the message. Not caught, it logs no other
-- error line: Engine::uncaught_log_error.
local function LogError(Message, Given, Position)
	local Text, Template, Context = LogLine("error", Message, Given, Position)
	local Thrown = {}
	Thrown.message = Text
	Thrown.stack = R.LogStack(debug.traceback())
	Thrown.template = Template
	Thrown.context = if Context then LogContext(Context) else {}
	setmetatable(Thrown, { __tostring = LogErrorText })
	error(Thrown, 0)
end

for Member, Kind in { Output = "print", Info = "info", Warn = "warn" } do
	Method("LogService", Member, Priced(function(Self, Message, Context)
		LogSelf(Self, Member)
		LogLine(Kind, Message, Context, 2)
	end))
end
Method("LogService", "Error", Priced(function(Self, Message, Context)
	LogSelf(Self, "Error")
	LogError(Message, Context, 2)
end))
-- Log(messageType, message, context): an EnumItem of MessageType, its name or its value (measured: "MessageInfo" and 1 are
-- MessageInfo; another enum's item "Unable to cast Material to MessageType", 7 "Unable to cast double to MessageType")
local LogKinds = { MessageOutput = "print", MessageInfo = "info", MessageWarning = "warn", MessageError = "error" }
Method("LogService", "Log", Priced(function(Self, MessageType, Message, Context)
	LogSelf(Self, "Log")
	if MessageType == nil then
		error("Argument 2 missing or nil", 0)
	end
	local Item = nil
	for _, Candidate in Enum.MessageType:GetEnumItems() do
		if Candidate == MessageType or Candidate.Name == MessageType or Candidate.Value == MessageType then
			Item = Candidate
			break
		end
	end
	if not Item then
		local Type = typeof(MessageType)
		local Name = if Type == "number" then "double" elseif Type == "EnumItem" then tostring(MessageType.EnumType) else Type
		error(`Unable to cast {Name} to MessageType`, 0)
	end
	if Item.Name == "MessageError" then
		LogError(Message, Context, 3)
	end
	LogLine(LogKinds[Item.Name], Message, Context, 3)
end))

-- A BindableFunction's callback, as a client runs it: at the bottom of a thread of its own, its outcome handed to Finished as
-- xpcall packs it (lua/bindable.rs run_own: no level above the callback, the thread reused while its runs return, its memory
-- category the setting thread's, an error raised before it first yields logged as an uncaught error is, with its own stack).
-- The caller gets the error as text: a string's own, any other value's empty. Measured in a 0.747 playing client and server:
-- tools/parity/probes/loopback/remote_callback_errors.luau (its BindableFunction rows), engine_threads.luau. (Here at the end:
-- the stubs above keep their line numbers.)
local RunOwn = R.RunOwn
local function Answer(Outcome)
	if Outcome[1] then
		return table.unpack(Outcome, 2, Outcome.n)
	end
	error(if type(Outcome[2]) == "string" then Outcome[2] else "", 0)
end
-- BindableFunction:Invoke: the caller waits for its callback to end (when it yielded, resumed at the next deferred point after
-- the callback's thread ends: R.Defer)
R.InvokeOwn = function(Self, Callback, ...)
	local Caller = coroutine.running()
	local Outcome, Waiting
	RunOwn(Self, Callback, function(Ended)
		Outcome = Ended
		if Waiting then
			R.Defer(Caller)
		end
	end, ...)
	if not Outcome then
		Waiting = true
		coroutine.yield()
	end
	return Answer(Outcome)
end
-- BindableFunction:Invoke from a thread that cannot yield (a metamethod, a sort comparator): the call returns what the
-- callback returns when it ends without yielding, else raises `thread is not yieldable` while the callback goes on (a call with
-- no handler raises it too). Measured in a 0.747 playing client and server: tools/parity/probes/yield_refused.luau.
R.InvokeApart = function(Self, Callback, ...)
	if Self:GetActor() then
		Charge(Cost.ActorInvoke)
	end
	local Outcome
	RunOwn(Self, Callback, function(Ended)
		Outcome = Ended
	end, R.CopiedArguments(...))
	if not Outcome then
		error("thread is not yieldable", 0)
	end
	return R.InvokeCopied(Answer(Outcome))
end

-- A method's ContentId argument (a string, as it is): nil raises "Argument 1 missing or nil", anything else "Unable to cast <value>
-- to ContentId", with "double", "bool", "Array" or "Dictionary", "Function" or an item's enum for the value (measured, 0.747, a
-- playing client and a server: GetAssetFetchStatus, GetAssetFetchStatusChangedSignal, TextService:GetFamilyInfoAsync,
-- KeyframeSequenceProvider:GetKeyframeSequenceAsync, AnimationClipProvider:GetAnimationClipAsync and InsertService:CreateMeshPartAsync
-- alike, tools/parity/probes/service_answers.luau)
ContentIdArg = function(Value, Index)
	if Value == nil then
		error(`Argument {Index or 1} missing or nil`, 0)
	elseif type(Value) == "string" then
		return Value
	end
	local Kind = typeof(Value)
	if Kind == "number" then
		Kind = "double"
	elseif Kind == "boolean" then
		Kind = "bool"
	elseif Kind == "table" then
		Kind = if #Value > 0 or next(Value) == nil then "Array" else "Dictionary"
	elseif Kind == "function" then
		Kind = "Function"
	elseif Kind == "EnumItem" then
		Kind = tostring(Value.EnumType)
	end
	error(`Unable to cast {Kind} to ContentId`, 0)
end

-- (one signal for each id, never fired: R.FetchStatusSignal at the end of this file)
Method("ContentProvider", "GetAssetFetchStatusChangedSignal", function(_, ContentId)
	ContentIdArg(ContentId)
	return R.FetchStatusSignal(ContentId)
end)
-- InsertService:CreateMeshPartAsync loads a mesh asset (not emulated) once its id is read
Method("InsertService", "CreateMeshPartAsync", function(_, MeshId)
	ContentIdArg(MeshId)
	Missing("InsertService", "CreateMeshPartAsync")
end)


-- Model methods, measured in a Studio 0.747 playing client and its server (tools/parity/probes/model_methods.luau). The lines
-- above refer to these through R, so no line above moved (engine Luau line numbers are in traces scripts see).

-- A primary-less model's pivot while unset (GetPivotInner): the bounding box about the part whose box across the world's axes
-- covers the most ground (its X by Z extent; the first of equal ones), oriented as that part: 26 models of two and three turned
-- parts, one made larger along one axis or another, matched to the bit (no part's own size, volume or face area does).
R.PivotBasis = function(Self)
	local Best, Most = nil, -1
	local function Visit(Node)
		if Node:IsA("BasePart") then
			local Frame, Size = RawGet(Node, "CFrame"), RawGet(Node, "Size")
			local _, _, _, R00, R01, R02, _, _, _, R20, R21, R22 = Frame:GetComponents()
			local Ground = (math.abs(R00) * Size.X + math.abs(R01) * Size.Y + math.abs(R02) * Size.Z) * (math.abs(R20) * Size.X + math.abs(R21) * Size.Y + math.abs(R22) * Size.Z)
			if Ground > Most then
				Best, Most = Frame, Ground
			end
		end
		for _, Child in Children(Node) do
			Visit(Child)
		end
	end
	Visit(Self)
	return Best
end

-- How far MoveTo raises a model so no part of it overlaps a part that can collide (measured: a model moved into a floor stands on
-- it, a turned part by its box across the world's axes, and onto what it then meets in turn; parts with CanCollide off are passed
-- through). A client lands a few float steps higher (62.0000076 for 62): not followed.
local function Lift(Self, Parts, Delta)
	local Params = OverlapParams.new()
	Params.FilterType = Enum.RaycastFilterType.Exclude
	Params.FilterDescendantsInstances = { Self }
	Params.RespectCanCollide = true
	local Root = Self:FindFirstAncestorWhichIsA("WorldRoot") or workspace
	local function Reach(Frame, Size)
		local _, _, _, R00, R01, R02, R10, R11, R12, R20, R21, R22 = Frame:GetComponents()
		return Vector3.new(math.abs(R00) * Size.X + math.abs(R01) * Size.Y + math.abs(R02) * Size.Z, math.abs(R10) * Size.X + math.abs(R11) * Size.Y + math.abs(R12) * Size.Z, math.abs(R20) * Size.X + math.abs(R21) * Size.Y + math.abs(R22) * Size.Z) / 2
	end
	local Raised = 0
	for _ = 1, 64 do
		local Need = 0
		for _, Part in Parts do
			local Frame = RawGet(Part, "CFrame") + Delta + Vector3.new(0, Raised, 0)
			local Half = Reach(Frame, RawGet(Part, "Size"))
			local Bottom = Frame.Position.Y - Half.Y
			for _, Hit in Root:GetPartBoundsInBox(CFrame.new(Frame.Position), Half * 2, Params) do
				local HitFrame = RawGet(Hit, "CFrame")
				Need = math.max(Need, HitFrame.Position.Y + Reach(HitFrame, RawGet(Hit, "Size")).Y - Bottom)
			end
		end
		if Need <= 0 then
			break
		end
		Raised += Need
	end
	return Raised
end

-- Model:MoveTo and Model:move: the pivot's position to Position, every part translated by the same amount (their rotations kept to
-- the bit), raised clear of what can collide (Lift). A model whose pivot is unset then stores its new box's pivot moved once more
-- (the old pivot plus twice the move, measured: the box moves with the parts, then the move is added); a stored pivot moves once;
-- a PrimaryPart's moves with it. Nothing for a model outside the DataModel or with no parts. Priced as PivotTo (not measured).
R.MoveModel = Priced(function(Self, Position)
	local Parts, Others, Models = {}, 0, {}
	for _, Descendant in Self:GetDescendants() do
		if Descendant:IsA("BasePart") then
			table.insert(Parts, Descendant)
		else
			Others += 1
			if Descendant:IsA("Model") then
				table.insert(Models, Descendant)
			end
		end
	end
	if #Parts == 0 then
		Charge(Cost.PivotBase + Others * Cost.PivotDescendant)
		return
	end
	local Delta = Position - GetPivotInner(Self).Position
	Delta += Vector3.new(0, Lift(Self, Parts, Delta), 0)
	Together(function()
		for _, Part in Parts do
			Part.CFrame = RawGet(Part, "CFrame") + Delta
		end
		for _, Inner in Models do
			local Stored = StoredPivot(Inner)
			if Stored then
				StorePivot(Inner, Stored + Delta)
			end
		end
	end)
	Charge(Cost.PivotBase + #Parts * (if Self:IsDescendantOf(workspace) then Cost.PivotPart else Cost.PivotLoose) + Others * Cost.PivotDescendant)
	if not (Self:IsA("Model") and RawGet(Self, "PrimaryPart")) then
		StorePivot(Self, GetPivotInner(Self) + Delta)
	end
end)
Method("Model", "move", function(Self, Position)
	if Self:IsDescendantOf(game) then R.MoveModel(Self, Position) end
end)
-- The old box methods: GetModelCFrame is GetBoundingBox's CFrame, GetModelSize its size (an empty model's the identity and zero)
Method("Model", "GetModelCFrame", Priced(function(Self)
	Charge(Cost.ModelQuery)
	return (GetBoundingBoxInner(Self))
end))
Method("Model", "GetModelSize", Priced(function(Self)
	Charge(Cost.ModelExtentsQuery)
	local _, Size = GetBoundingBoxInner(Self)
	return Size
end))
-- Model:PivotTo puts each part at Target * Pivot:ToObjectSpace(it), the primary part too (so a model's pivot after it carries the
-- float steps of that product), and keeps each part's offset from the pivot: a later PivotTo, while the pivot is the one the last
-- left and the part is where it put it, uses the kept offset, so PivotTo to the same target again changes nothing (measured to the
-- bit, 0.747: models with and without a PrimaryPart, a PivotOffset, three targets, each twice; a Part's PivotTo composes as before).
local PivotOffsets = setmetatable({}, { __mode = "k" })
local PivotsLeft = setmetatable({}, { __mode = "k" })
-- (a call whose pivot is not the one the last left forgets the offsets)
R.PivotBegin = function(Self, Pivot)
	if PivotsLeft[Self] ~= Pivot then
		PivotOffsets[Self] = setmetatable({}, { __mode = "k" })
	end
	PivotsLeft[Self] = nil
end
R.PivotPlace = function(Self, Item, Target, Pivot, Current)
	local Kept = PivotOffsets[Self]
	if not Kept then
		Kept = setmetatable({}, { __mode = "k" })
		PivotOffsets[Self] = Kept
	end
	local Entry = Kept[Item]
	local Offset = if Entry and Entry.At == Current then Entry.Offset else Pivot:ToObjectSpace(Current)
	Kept[Item] = { Offset = Offset, Placed = true }
	return Target * Offset
end
-- (once the call's parts are written and settled: where each part is now, as stored, and the pivot the call leaves, the target or
-- with a PrimaryPart its pivot where the call put it; a target written orthonormalized keeps its offset: cframe_writes.luau)
R.PivotDone = function(Self, Target)
	for Item, Entry in PivotOffsets[Self] or {} do
		if Entry.Placed then
			Entry.At = if Item:IsA("BasePart") then RawGet(Item, "CFrame") else StoredPivot(Item)
			Entry.Placed = nil
		end
	end
	PivotsLeft[Self] = if Self:IsA("Model") and RawGet(Self, "PrimaryPart") then GetPivotInner(Self) else Target
end
-- SetIdentityOrientation and ResetOrientationToIdentity change nothing a script sees (a model turned between them stays turned)
for _, Name in { "SetIdentityOrientation", "ResetOrientationToIdentity" } do
	Method("Model", Name, function() end)
end

-- A method argument as a client's methods cast it (measured in a 0.747 playing client and a server: tools/parity/probes/
-- arg_checks.luau, humanoid_description_emotes.luau, primary_part_cframe.luau): the arguments are cast from the last to the
-- first, and a value that is no argument's type raises "Unable to cast <value> to <type>", the value named "double", "bool",
-- "Array" (a table with an array part, or empty) or "Dictionary", "Function", an item's enum, "null" for a userdata of no type,
-- else its type; nil, where the argument has no default, "Argument <n> missing or nil". An Object takes nil and any Instance
-- ("Unable to cast value to Object" for anything else); a string takes a number as its text (StringParam).
local CastName = function(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		return "double"
	elseif Kind == "boolean" then
		return "bool"
	elseif Kind == "table" then
		return if rawlen(Value) > 0 or next(Value) == nil then "Array" else "Dictionary"
	elseif Kind == "function" then
		return "Function"
	elseif Kind == "EnumItem" then
		return tostring(Value.EnumType)
	elseif Kind == "CFrame" then
		return "CoordinateFrame"
	elseif Kind == "userdata" then
		return "null"
	end
	return Kind
end
local Cast = {}
function Cast.Object(Value)
	if Value ~= nil and typeof(Value) ~= "Instance" then
		error("Unable to cast value to Object", 0)
	end
	return Value
end
function Cast.string(Value, Index, Default)
	if Value == nil and Default ~= nil then
		return Default
	end
	return StringParam.Read(Value, Index)
end
-- (a number, or a string that reads as one; int and int64 round half away from zero: AddEmote's 1.5 is 2, 2.5 is 3)
local function Number(Value, Index, Default, Type)
	if Value == nil then
		if Default ~= nil then
			return Default
		end
		error(`Argument {Index} missing or nil`, 0)
	end
	local Read = if type(Value) == "number" then Value elseif type(Value) == "string" then tonumber(Value) else nil
	if Read == nil then
		error(`Unable to cast {CastName(Value)} to {Type}`, 0)
	end
	if Type == "int" or Type == "int64" then
		Read = if Read < 0 then -math.floor(-Read + 0.5) else math.floor(Read + 0.5)
	end
	return Read
end
for _, Type in { "double", "float", "int", "int64" } do
	Cast[Type] = function(Value, Index, Default)
		return Number(Value, Index, Default, Type)
	end
end
-- (a table without an array part; its keys strings or numbers, else "Invalid table key type used", as TweenService:Create's goals)
function Cast.Dictionary(Value, Index, Default)
	if Value == nil then
		if Default ~= nil then
			return Default
		end
		error(`Argument {Index} missing or nil`, 0)
	elseif type(Value) ~= "table" or rawlen(Value) > 0 then
		error(`Unable to cast {CastName(Value)} to Dictionary`, 0)
	end
	for Key in Value do
		if type(Key) ~= "string" and type(Key) ~= "number" then
			error("Invalid table key type used", 0)
		end
	end
	return Value
end
function Cast.Array(Value, Index)
	if Value == nil then
		error(`Argument {Index} missing or nil`, 0)
	elseif type(Value) ~= "table" or (rawlen(Value) == 0 and next(Value) ~= nil) then
		error(`Unable to cast {CastName(Value)} to Array`, 0)
	end
	return Value
end
-- (an item of the enum, an item's name, or a number cut toward zero to an item's value: lua/convert.rs coerce_enum_arg)
local function CastEnum(EnumName, Value, Index, Default)
	if Value == nil then
		if Default ~= nil then
			return Default
		end
		error(`Argument {Index} missing or nil`, 0)
	end
	local Type = Enum[EnumName]
	local Kind = typeof(Value)
	if Kind == "EnumItem" and Value.EnumType == Type then
		return Value
	end
	for _, Item in Type:GetEnumItems() do
		if (Kind == "string" and Item.Name == Value) or (Kind == "number" and Value == Value and math.abs(Value) ~= math.huge and Item.Value == (if Value < 0 then math.ceil(Value) else math.floor(Value))) then
			return Item
		end
	end
	error(`Unable to cast {CastName(Value)} to {EnumName}`, 0)
end
-- A method's arguments by their types (`{ Type, Default }`, an enum's name for its items), cast from the last to the first
local function CastAll(Types, ...)
	local Values = table.pack(...)
	for Index = #Types, 1, -1 do
		local Type, Default = Types[Index][1], Types[Index][2]
		local Caster = Cast[Type]
		if Caster then
			Values[Index] = Caster(Values[Index], Index, Default)
		else
			Values[Index] = CastEnum(Type, Values[Index], Index, Default)
		end
	end
	return table.unpack(Values, 1, #Types)
end

-- Model:GetPrimaryPartCFrame and SetPrimaryPartCFrame (measured in a 0.747 playing client and a server:
-- tools/parity/probes/primary_part_cframe.luau): the CFrame is cast first ("Argument 1 missing or nil", "Unable to cast string to
-- CoordinateFrame"), then a model whose PrimaryPart is unset, or no longer under the model (taken out or destroyed), raises
-- "Model:SetPrimaryPartCFrame() failed because no PrimaryPart has been set, or the PrimaryPart no longer exists. Please set
-- Model.PrimaryPart before using this." (GetPrimaryPartCFrame's names itself). They read and set the primary part's own CFrame,
-- not the model's pivot: its PivotOffset and the model's WorldPivot play no part, and the part is put at the CFrame exactly, the
-- model's other parts moved with it as one (priced as PivotTo's move).
PrimaryPartCFrame = Priced(function(Self, Count, Target)
	local Name = if Count then "SetPrimaryPartCFrame" else "GetPrimaryPartCFrame"
	if Count then
		if Target == nil then
			error("Argument 1 missing or nil", 0)
		elseif typeof(Target) ~= "CFrame" then
			error(`Unable to cast {CastName(Target)} to CoordinateFrame`, 0)
		end
	end
	local Primary = RawGet(Self, "PrimaryPart")
	if not Primary or not Primary:IsDescendantOf(Self) then
		error(`Model:{Name}() failed because no PrimaryPart has been set, or the PrimaryPart no longer exists. Please set Model.PrimaryPart before using this.`, 0)
	end
	if not Count then
		Charge(Cost.ModelQuery)
		return Primary.CFrame
	end
	local Delta = Target * Primary.CFrame:Inverse()
	local Stored = StoredPivot(Self)
	local Moved, Others = 0, 0
	Together(function()
		Moved, Others = MoveDescendants(Self, Delta)
		Primary.CFrame = Target
	end)
	Charge(Cost.PivotBase + Moved * (if Self:IsDescendantOf(workspace) then Cost.PivotPart else Cost.PivotLoose) + Others * Cost.PivotDescendant)
	if Stored then
		StorePivot(Self, Delta * Stored)
	end
	return nil
end)

-- HumanoidDescription's emotes (measured in a 0.747 playing client and a server: tools/parity/probes/humanoid_description_emotes.luau),
-- kept in its EmotesDataInternal and EquippedEmotesDataInternal (which a clone copies and a script cannot read) as JSON:
-- - The emotes are a dictionary of names, each a list of asset ids: ids are numbers (other values are dropped, a name left with
--   none is dropped), rounded half away from zero, each once, in ascending order; a number for a name is its text. GetEmotes gives
--   a new copy each call. SetEmotes replaces them (a table with an array part "Unable to cast Array to Dictionary"); AddEmote adds
--   an id to a name's list; RemoveEmote takes a name out (one it lacks changes nothing). Each fires EmotesChanged with a copy,
--   SetEmotes and AddEmote even when nothing changed, RemoveEmote only when it took one out.
-- - The equipped emotes are a list of { Slot, Name } in slot order: SetEquippedEmotes takes a list (a dictionary "Unable to cast
--   Dictionary to Array") of names, which take the slots 1, 2, ... in the order they come, and { Slot = <number>, Name = <string> }
--   tables (the slot rounded; any number), the later of two for one slot kept; other entries are dropped, and no slot is checked
--   against the emotes. It fires EquippedEmotesChanged with a copy every time.
local function EmoteData(Self, Property)
	local Text = RawGet(Self, Property)
	if type(Text) == "string" and Text ~= "" then
		local Ok, Data = R.JsonDecode(Text)
		if Ok and type(Data) == "table" then
			return Data
		end
	end
	return {}
end
local function Round(Value)
	return if Value < 0 then -math.floor(-Value + 0.5) else math.floor(Value + 0.5)
end
local function EmotesCopy(Emotes)
	local Copy = {}
	for Name, Ids in Emotes do
		Copy[Name] = table.clone(Ids)
	end
	return Copy
end
local function SetEmoteData(Self, Emotes)
	local Kept = {}
	for Name, Ids in Emotes do
		if #Ids > 0 then
			Kept[Name] = Ids
		end
	end
	RawSet(Self, "EmotesDataInternal", R.JsonEncode(Kept), true)
	R.FireEvent(Self, "EmotesChanged", EmotesCopy(Kept))
end
local function EmoteIds(List)
	local Seen, Ids = {}, {}
	if type(List) == "table" then
		for Index = 1, rawlen(List) do
			local Id = rawget(List, Index)
			if type(Id) == "number" and Id == Id then
				Id = Round(Id)
				if not Seen[Id] then
					Seen[Id] = true
					table.insert(Ids, Id)
				end
			end
		end
	end
	table.sort(Ids)
	return Ids
end
Method("HumanoidDescription", "GetEmotes", function(Self)
	return EmotesCopy(EmoteData(Self, "EmotesDataInternal"))
end)
Method("HumanoidDescription", "SetEmotes", function(Self, ...)
	local Emotes = CastAll({ { "Dictionary" } }, ...)
	local Kept = {}
	for Name, Ids in Emotes do
		Kept[tostring(Name)] = EmoteIds(Ids)
	end
	SetEmoteData(Self, Kept)
end)
Method("HumanoidDescription", "AddEmote", function(Self, ...)
	local Name, Id = CastAll({ { "string" }, { "int64" } }, ...)
	local Emotes = EmoteData(Self, "EmotesDataInternal")
	local Ids = Emotes[Name] or {}
	table.insert(Ids, Id)
	Emotes[Name] = EmoteIds(Ids)
	SetEmoteData(Self, Emotes)
end)
Method("HumanoidDescription", "RemoveEmote", function(Self, ...)
	local Name = CastAll({ { "string" } }, ...)
	local Emotes = EmoteData(Self, "EmotesDataInternal")
	if Emotes[Name] ~= nil then
		Emotes[Name] = nil
		SetEmoteData(Self, Emotes)
	end
end)
local function EquippedList(Self)
	local List = {}
	for _, Entry in EmoteData(Self, "EquippedEmotesDataInternal") do
		table.insert(List, { Slot = Entry[1], Name = Entry[2] })
	end
	return List
end
Method("HumanoidDescription", "GetEquippedEmotes", function(Self)
	return EquippedList(Self)
end)
Method("HumanoidDescription", "SetEquippedEmotes", function(Self, ...)
	local Given = CastAll({ { "Array" } }, ...)
	local Slots, Next = {}, 0
	for Index = 1, rawlen(Given) do
		local Entry = rawget(Given, Index)
		if type(Entry) == "string" then
			Next += 1
			Slots[Next] = Entry
		elseif type(Entry) == "table" and type(rawget(Entry, "Slot")) == "number" and type(rawget(Entry, "Name")) == "string" then
			Slots[Round(rawget(Entry, "Slot"))] = rawget(Entry, "Name")
		end
	end
	local Order = {}
	for Slot in Slots do
		table.insert(Order, Slot)
	end
	table.sort(Order)
	local Stored = {}
	for _, Slot in Order do
		table.insert(Stored, { Slot, Slots[Slot] })
	end
	RawSet(Self, "EquippedEmotesDataInternal", R.JsonEncode(Stored), true)
	R.FireEvent(Self, "EquippedEmotesChanged", EquippedList(Self))
end)

-- RemoteFunction:InvokeClient in a client's run (a loopback run's is loopback.luau's, a server's server.luau's): the arguments are
-- converted first (R.Sendable), then the player checked ("Argument 1 missing or nil", "Argument 1 is invalid: expected 'Player'
-- instance type"), then a remote in the DataModel raises "InvokeClient can only be called from the server", then a thread that cannot
-- yield "thread is not yieldable"; a parentless one waits for good (a 0.747 playing client: tools/parity/probes/remote_call_order.luau)
Method("RemoteFunction", "InvokeClient", function(Self, Player, ...)
	R.Sendable(select("#", ...) + 2, ...) if Player == nil then
		error("Argument 1 missing or nil", 0)
	elseif typeof(Player) ~= "Instance" or not Player:IsA("Player") then
		error("Argument 1 is invalid: expected 'Player' instance type", 0)
	elseif Self.Parent then
		error("InvokeClient can only be called from the server", 0)
	end
	if not coroutine.isyieldable() then error("thread is not yieldable", 0) end return coroutine.yield()
end)

-- AnalyticsService's Log methods (measured in a 0.747 playing client and a server: tools/parity/probes/arg_checks.luau): their
-- arguments cast by their API types from the last to the first, then the warning "AnalyticsService can only be executed on the
-- game server." on a client and "AnalyticsService: Invalid status." on a server that is no published game's. (LogJourneyEvent and
-- the three LogProgression*Event ones were not called there: they take the same casts by their types.)
local AnalyticsTypes = {
	LogCustomEvent = { { "Object" }, { "string" }, { "double", 1 }, { "Dictionary", {} } },
	LogEconomyEvent = { { "Object" }, { "AnalyticsEconomyFlowType" }, { "string" }, { "float" }, { "float" }, { "string" }, { "string", "" }, { "Dictionary", {} } },
	LogFunnelStepEvent = { { "Object" }, { "string" }, { "string", "" }, { "int", 1 }, { "string", "" }, { "Dictionary", {} } },
	LogJourneyEvent = { { "Object" }, { "string" }, { "string" }, { "string", "" }, { "Dictionary", {} } },
	LogOnboardingFunnelStepEvent = { { "Object" }, { "int" }, { "string", "" }, { "Dictionary", {} } },
	LogProgressionCompleteEvent = { { "Object" }, { "string" }, { "int" }, { "string", "" }, { "Dictionary", {} } },
	LogProgressionEvent = { { "Object" }, { "string" }, { "AnalyticsProgressionType" }, { "int" }, { "string", "" }, { "Dictionary", {} } },
	LogProgressionFailEvent = { { "Object" }, { "string" }, { "int" }, { "string", "" }, { "Dictionary", {} } },
	LogProgressionStartEvent = { { "Object" }, { "string" }, { "int" }, { "string", "" }, { "Dictionary", {} } },
}
for Name, Types in AnalyticsTypes do
	Method("AnalyticsService", Name, function(_, ...)
		CastAll(Types, ...)
		warn(if game:GetService("RunService"):IsServer() then "AnalyticsService: Invalid status." else "AnalyticsService can only be executed on the game server.")
	end)
end

-- Chat:Chat(partOrCharacter, message, color) (measured likewise): its casts (a color is a ChatColor, Blue when left off), then
-- "partOrCharacter must be non-nil", "partOrCharacter must be in the Workspace", "message must be non-empty". The bubble it
-- shows is a client's GUI, not emulated.
Method("Chat", "Chat", function(_, ...)
	local Part, Message = CastAll({ { "Object" }, { "string" }, { "ChatColor", Enum.ChatColor.Blue } }, ...)
	if Part == nil then
		error("partOrCharacter must be non-nil", 0)
	elseif not Part:IsDescendantOf(workspace) then
		error("partOrCharacter must be in the Workspace", 0)
	elseif Message == "" then
		error("message must be non-empty", 0)
	end
	Missing("Chat", "Chat")
end)

-- MarketplaceService:PromptSubscriptionPurchase(user, subscriptionId) (measured likewise): its casts, then "First argument to
-- PromptSubscriptionPurchase must be a player" for anything that is no Player, "Invalid SubscriptionId" for an id that does not
-- start "EXP-" ("EXP-1" was taken, "5" and a table's text refused; the order of the two checks was not measured). The prompt is
-- a client's GUI, not emulated.
Method("MarketplaceService", "PromptSubscriptionPurchase", function(_, ...)
	local User, Id = CastAll({ { "Object" }, { "string" } }, ...)
	if User == nil or not User:IsA("Player") then
		error("First argument to PromptSubscriptionPurchase must be a player", 0)
	elseif string.sub(Id, 1, 4) ~= "EXP-" then
		error("Invalid SubscriptionId", 0)
	end
	Missing("MarketplaceService", "PromptSubscriptionPurchase")
end)

-- KeyframeSequenceProvider's and AnimationClipProvider's GetAnimationsAsync(userId) and the deprecated GetAnimations (measured,
-- a 0.747 playing client and server: tools/parity/probes/animation_members.luau): the id is cast as a "User" (an int64: a number,
-- a boolean or a string of a number; nil "Argument 1 missing or nil", other values "Unable to cast string to User", "... Array to
-- User", "... Material to User"), then a thread that cannot yield gets "thread is not yieldable", then the web answers "HTTP 403"
-- after a wait (WebAnswerWait: the time the requests took there). AnimationClipProvider:GetClipEvaluatorAsync casts its ContentId
-- and raises "AnimationClipProvider:GetClipEvaluatorAsync is not currently enabled" at once, also where the thread cannot yield.
local WebAnswerWait = 0.2
local function UserArg(Value)
	local Kind = type(Value)
	if Value == nil then
		error("Argument 1 missing or nil", 0)
	elseif Kind == "number" or Kind == "boolean" or (Kind == "string" and tonumber(Value)) then
		return Value
	elseif Kind == "table" then
		Kind = if #Value > 0 or next(Value) == nil then "Array" else "Dictionary"
	elseif Kind == "function" then
		Kind = "Function"
	elseif typeof(Value) == "EnumItem" then
		Kind = tostring(Value.EnumType)
	else
		Kind = typeof(Value)
	end
	error(`Unable to cast {Kind} to User`, 0)
end
for _, Provider in { "KeyframeSequenceProvider", "AnimationClipProvider" } do
	for _, Name in { "GetAnimationsAsync", "GetAnimations" } do
		Method(Provider, Name, function(_, UserId)
			UserArg(UserId)
			if not coroutine.isyieldable() then
				error("thread is not yieldable", 0)
			end
			task.wait(WebAnswerWait)
			error("HTTP 403", 0)
		end)
	end
end
Method("AnimationClipProvider", "GetClipEvaluatorAsync", function(_, AssetId)
	ContentIdArg(AssetId)
	error("AnimationClipProvider:GetClipEvaluatorAsync is not currently enabled", 0)
end)

-- Engine data methods (parity package U, measured in a 0.747 playing client and a server, each probe twice:
-- tools/parity/probes/engine_data_methods.luau). The casts are a client's, from the last argument to the first (CastAll).
function Cast.bool(Value, Index, Default)
	if Value == nil then
		if Default ~= nil then
			return Default
		end
		error(`Argument {Index} missing or nil`, 0)
	elseif type(Value) ~= "boolean" then
		error(`Unable to cast {CastName(Value)} to bool`, 0)
	end
	return Value
end
function Cast.Function(Value, Index)
	if Value == nil then
		error(`Argument {Index} missing or nil`, 0)
	elseif type(Value) ~= "function" then
		error(`Unable to cast {CastName(Value)} to Function`, 0)
	end
	return Value
end
function Cast.Vector3(Value, Index)
	if Value == nil then
		error(`Argument {Index} missing or nil`, 0)
	elseif typeof(Value) ~= "Vector3" then
		error(`Unable to cast {CastName(Value)} to Vector3`, 0)
	end
	return Value
end
-- (an Object a method needs: nil is "Argument n missing or nil")
function Cast.Instance(Value, Index)
	if Value == nil then
		error(`Argument {Index} missing or nil`, 0)
	end
	return Cast.Object(Value)
end
local function OnServer()
	return not game:GetService("RunService"):IsClient()
end
local function Float32(Value)
	return (string.unpack("f", string.pack("f", Value)))
end

-- ContentProvider's encryption keys and encrypted assets: taken and forgotten (ListEncryptedAssets lists none, even after a
-- register); ids are ContentIds, keys strings. Preload (deprecated) fetches nothing here.
Method("ContentProvider", "ListEncryptedAssets", function()
	return {}
end)
for _, Name in { "RegisterDefaultEncryptionKey", "RegisterDefaultSessionKey" } do
	Method("ContentProvider", Name, function(_, ...)
		CastAll({ { "string" } }, ...)
	end)
end
Method("ContentProvider", "UnregisterDefaultEncryptionKey", function() end)
Method("ContentProvider", "UnregisterEncryptedAsset", function(_, Id)
	ContentIdArg(Id)
end)
Method("ContentProvider", "Preload", function(_, Id)
	ContentIdArg(Id)
end)
for _, Name in { "RegisterEncryptedAsset", "RegisterSessionEncryptedAsset" } do
	Method("ContentProvider", Name, function(_, Id, Key)
		Cast.string(Key, 2)
		ContentIdArg(Id)
	end)
end
-- GetAssetFetchStatusChangedSignal: one signal for each id, named "<id>AssetFetchStatusChanged" with rbxassetid:// left off
-- (an rbxthumb:// id gains "&filters=" as a client writes it); an http(s) URL is no content id. ree fetches no asset, so the
-- status of every id stays None and the signal never fires (a client fires it when a fetch it makes ends).
local FetchSignals = {}
R.FetchStatusSignal = function(Id)
	local Key = string.gsub(Id, "^rbxassetid://", "")
	if string.find(Id, "^https?://") then
		error(`ContentProvider:GetAssetFetchStatusChangedSignal: {Id} is not a valid content id`, 0)
	elseif string.find(Key, "^rbxthumb://") and not string.find(Key, "filters=", 1, true) then
		Key ..= "&filters="
	end
	local Signal = FetchSignals[Key]
	if not Signal then
		Signal = R.NewSignal(Key .. "AssetFetchStatusChanged")
		FetchSignals[Key] = Signal
	end
	return Signal
end

-- CollectionService: GetCollection (deprecated for GetTagged) answers an empty list, and GetCollections an instance's
-- collections, the CreateCollection handles it is a member of (R.CollectionHandles: this file's end block).
Method("CollectionService", "GetCollection", function(_, ...)
	CastAll({ { "string" } }, ...)
	return {}
end)
Method("CollectionService", "GetCollections", function(_, ...)
	CastAll({ { "Instance" } }, ...)
	return R.CollectionHandles((...))
end)

-- ContextActionService: no tool, so no tool icon; BindActivate binds input, the host's input layer's (it reports itself after its
-- casts); a server refuses BindActivate, UnbindActivate and BindActionToInputTypes (BindAction's message), which a client
-- treats as BindAction.
Method("ContextActionService", "GetCurrentLocalToolIcon", function()
	return ""
end)
for _, Name in { "BindActivate", "UnbindActivate" } do
	Method("ContextActionService", Name, function(_, ...)
		CastAll({ { "UserInputType" } }, ...)
		if OnServer() then
			error(`ContextActionService:{Name} can only be called from a local script`, 0)
		elseif Name == "BindActivate" then
			Missing("ContextActionService", "BindActivate")
		end
	end)
end
Method("ContextActionService", "BindActionToInputTypes", function(Self, Name, Function, Touch, ...)
	CastAll({ { "string" }, { "Function" }, { "bool" } }, Name, Function, Touch)
	if OnServer() then
		error("ContextActionService:BindAction can only be called from a local script", 0)
	end
	return Self:BindAction(Name, Function, Touch, ...)
end)

-- LogService: ClearOutput clears a developer console ree has none of; GetLogHistoryAsync asks the web for a user's log (after its
-- casts it reports itself); GetLogger(name) answers one Logger for each name (typeof "Object", tostring "Logger"), whose Info,
-- Warn and Error log their message as MessageInfo, MessageWarning and MessageError (Error raises nothing), Log(messageType,
-- message) as that type, a number message as its text, further arguments ignored; its methods count the logger as their first
-- argument and cast from the first ("Argument 2 missing or nil", "Unable to cast string to MessageType").
Method("LogService", "ClearOutput", function() end)
Method("LogService", "GetLogHistoryAsync", function(_, User)
	UserArg(User)
	Missing("LogService", "GetLogHistoryAsync")
	return {}
end)
local Loggers = {}
local LoggerKinds = { Info = "info", Warn = "warn", Error = "error" }
local LoggerTypes = { [Enum.MessageType.MessageOutput] = "print", [Enum.MessageType.MessageInfo] = "info", [Enum.MessageType.MessageWarning] = "warn", [Enum.MessageType.MessageError] = "error" }
Method("LogService", "GetLogger", function(_, Name)
	if type(Name) ~= "string" and type(Name) ~= "number" then
		error("missing argument #2 to 'GetLogger' (string expected)", ScriptLevel(2))
	end
	Name = tostring(Name)
	local Found = Loggers[Name]
	if Found then
		return Found
	end
	local Logger = newproxy(true)
	local Meta = getmetatable(Logger)
	local Members = { Name = Name }
	for Member, Kind in LoggerKinds do
		Members[Member] = function(_, Message)
			R.LogLine(Kind, StringParam.Read(Message, 2), nil)
			Charge(Cost.LogCall)
		end
	end
	Members.Log = function(_, Type, Message)
		Type = CastEnum("MessageType", Type, 2)
		R.LogLine(LoggerTypes[Type] or "print", StringParam.Read(Message, 3), nil)
		Charge(Cost.LogCall)
	end
	Meta.__index = function(_, Key)
		local Value = Members[Key]
		if Value == nil then
			error(`{Key} is not a valid member of Logger`, 0)
		end
		return Value
	end
	R.Register(Logger, "Object")
	Meta.__tostring = function()
		return "Logger"
	end
	Meta.__metatable = "The metatable is locked"
	Loggers[Name] = Logger
	return Logger
end)

-- Stats: every developer memory tag reads 0 on a playing client and a server; the total is the process's own figure, which ree
-- does not model (it reports itself and answers 0).
Method("Stats", "GetMemoryUsageMbForTag", function(_, ...)
	CastAll({ { "DeveloperMemoryTag" } }, ...)
	return 0
end)
Method("Stats", "GetTotalMemoryUsageMb", function()
	Missing("Stats", "GetTotalMemoryUsageMb")
	return 0
end)

-- Teams:RebalanceTeams moves no player of a client (not measured on a server with players); ReplicatedFirst's default
-- loading screen is a client's GUI, its removal nothing to see here (a server answers too); Lighting:GetMoonPhase reads 0.75
-- at every time of day and latitude.
Method("Teams", "RebalanceTeams", function()
	if OnServer() and #game:GetService("Players"):GetPlayers() > 0 then
		Missing("Teams", "RebalanceTeams")
	end
end)
Method("ReplicatedFirst", "RemoveDefaultLoadingScreen", function() end)
Method("Lighting", "GetMoonPhase", function()
	return 0.75
end)

-- SoundService's listener: Camera until set; SetListener(type, value) takes a CFrame for CFrame, a BasePart or Model for the
-- Object types ("SoundService:SetListener called with an incorrect argument." without one, "SoundService:SetListener value
-- given is not a valid CFrame when given Enum.ListenerType.CFrame" for anything else there), and GetListener answers it. (The
-- mixer's clock, GetMixerTime, is the audio device's: not emulated.)
Method("SoundService", "GetListener", function(Self)
	local Listener = Extra(Self).Listener
	if not Listener then
		return Enum.ListenerType.Camera, nil
	end
	return Listener[1], Listener[2]
end)
Method("SoundService", "SetListener", function(Self, Type, ...)
	Type = CastAll({ { "ListenerType" } }, Type)
	local Value = ...
	if Type ~= Enum.ListenerType.Camera then
		if Value == nil then
			error("SoundService:SetListener called with an incorrect argument.", 0)
		elseif Type == Enum.ListenerType.CFrame and typeof(Value) ~= "CFrame" then
			error("SoundService:SetListener value given is not a valid CFrame when given Enum.ListenerType.CFrame", 0)
		end
	else
		Value = nil
	end
	Extra(Self).Listener = { Type, Value }
end)

-- MaterialService: a material's base override is the material's name until SetBaseMaterialOverride names another (any text,
-- "" too); Neon, Glass, ForceField, Air and Water refuse ("GetBaseMaterialOverride failed: Material 'Neon' is not supported.").
-- GetMaterialVariant(material, name) finds a MaterialVariant of that name and base material anywhere under MaterialService, the
-- last one there of two (not under the Workspace); the material is an item, its name or its value. What an override draws is the
-- renderer's.
local UnsupportedMaterials = { Neon = true, Glass = true, ForceField = true, Air = true, Water = true }
local function OverrideMaterial(Name, ...)
	local Material = CastAll({ { "Material" } }, ...)
	if UnsupportedMaterials[Material.Name] then
		error(`{Name} failed: Material '{Material.Name}' is not supported.`, 0)
	end
	return Material
end
Method("MaterialService", "GetBaseMaterialOverride", function(Self, ...)
	local Material = OverrideMaterial("GetBaseMaterialOverride", ...)
	local Overrides = Extra(Self).Overrides
	return Overrides and Overrides[Material] or Material.Name
end)
Method("MaterialService", "SetBaseMaterialOverride", function(Self, Material, Name)
	Name = Cast.string(Name, 2)
	Material = OverrideMaterial("SetBaseMaterialOverride", Material)
	local Found = Extra(Self)
	Found.Overrides = Found.Overrides or {}
	Found.Overrides[Material] = Name
end)
Method("MaterialService", "GetMaterialVariant", function(Self, ...)
	local Material, Name = CastAll({ { "Material" }, { "string" } }, ...)
	local Found = nil
	for _, Descendant in Self:GetDescendants() do
		if Descendant:IsA("MaterialVariant") and Descendant.Name == Name and Descendant.BaseMaterial == Material then
			Found = Descendant
		end
	end
	return Found
end)

-- A Tool's Activate and Deactivate fire Activated and Deactivated when it is equipped (in a player's character) and enabled
-- (ManualActivationOnly does not stop them; a disabled tool's Activate fires nothing, its Deactivate taken alike, not measured);
-- unequipped, in the Workspace or in a model with a Humanoid that is no player's character, nothing.
local function Equipped(Tool)
	local Parent = Tool.Parent
	return Parent ~= nil and game:GetService("Players"):GetPlayerFromCharacter(Parent) ~= nil and RawGet(Tool, "Enabled") ~= false
end
Method("Tool", "Activate", function(Self)
	if Equipped(Self) then
		R.FireEvent(Self, "Activated")
	end
end)
Method("Tool", "Deactivate", function(Self)
	if Equipped(Self) then
		R.FireEvent(Self, "Deactivated")
	end
end)

-- TeleportOptions' teleport data: any value but nil, copied as BindableEvent:Fire copies it, at the write and for each read.
Method("TeleportOptions", "SetTeleportData", function(Self, Data)
	if Data == nil then
		error("Argument 1 missing or nil", 0)
	end
	Extra(Self).TeleportData = table.pack(R.Copied(Data))
end)
Method("TeleportOptions", "GetTeleportData", function(Self)
	local Data = Extra(Self).TeleportData
	if not Data then
		return nil
	end
	return R.Copied(Data[1])
end)

-- A playing client's DataStore options: metadata a dictionary, copied at the write and each read ({} until set; a server's are
-- server.luau's, loaded after this file, with the stores' own limits); the experimental features a dictionary, taken.
for _, Class in { "DataStoreSetOptions", "DataStoreIncrementOptions" } do
	Method(Class, "SetMetadata", function(Self, ...)
		local Attributes = CastAll({ { "Dictionary" } }, ...)
		Extra(Self).Metadata = R.Copied(Attributes)
	end)
	Method(Class, "GetMetadata", function(Self)
		return R.Copied(Extra(Self).Metadata or {})
	end)
end
Method("DataStoreOptions", "SetExperimentalFeatures", function(_, ...)
	CastAll({ { "Dictionary" } }, ...)
end)

-- WireframeHandleAdornment's lines, paths and texts are drawn, not read back: the calls cast their arguments (a list's points
-- Vector3s, else "Variant cast failed") and draw nothing here.
local function Points(Value)
	Cast.Array(Value, 1)
	for _, Point in Value do
		if typeof(Point) ~= "Vector3" then
			error("Variant cast failed", 0)
		end
	end
end
Method("WireframeHandleAdornment", "AddLine", function(_, ...)
	CastAll({ { "Vector3" }, { "Vector3" } }, ...)
end)
Method("WireframeHandleAdornment", "AddLines", function(_, Value)
	Points(Value)
end)
Method("WireframeHandleAdornment", "AddPath", function(_, Value, Loop)
	CastAll({ { "Array" }, { "bool" } }, Value, Loop)
	Points(Value)
end)
Method("WireframeHandleAdornment", "AddText", function(_, ...)
	CastAll({ { "Vector3" }, { "string" }, { "float", 12 } }, ...)
end)
Method("WireframeHandleAdornment", "Clear", function() end)
-- (a Beam's texture offset is drawn too)
Method("Beam", "SetTextureOffset", function(_, ...)
	CastAll({ { "float", 0 } }, ...)
end)
-- (a GeneratedFolder has no PrimaryPart to read)
Method("GeneratedFolder", "SetPrimaryPart", function(_, ...)
	CastAll({ { "Instance" } }, ...)
end)
-- (a Dialog's players are those talking with it: the Dialog GUI's, so none)
Method("Dialog", "GetCurrentPlayers", function()
	return {}
end)

-- CustomEvent / CustomEventReceiver: SetValue keeps a value between 0 and 1 (a float32; a boolean is 1 or 0, a numeric string its
-- number, NaN kept); a receiver reads its source's value (0 with none), and its SourceValueChanged fires with the value it reads
-- when that changes, by a SetValue or a new Source (attached to an event whose value is still 0, nothing; detached, 0). (A
-- SetValue to the same value fires nothing: not measured.)
local function EventValue(Event)
	return if Event then Extra(Event).Value or 0 else 0
end
local function Receivers(Event)
	local Found = Extra(Event)
	Found.Receivers = Found.Receivers or setmetatable({}, { __mode = "k" })
	return Found.Receivers
end
local AttachOrder = 0
ContentSetter("CustomEventReceiver", "Source", function(Self, Value)
	local Previous = RawGet(Self, "Source")
	if Previous then
		Receivers(Previous)[Self] = nil
	end
	if typeof(Value) == "Instance" and Value:IsA("CustomEvent") then
		AttachOrder += 1
		Receivers(Value)[Self] = AttachOrder
	end
	local Before = EventValue(if Previous and Previous:IsA("CustomEvent") then Previous else nil)
	local After = EventValue(if typeof(Value) == "Instance" and Value:IsA("CustomEvent") then Value else nil)
	if After ~= Before then
		R.FireEvent(Self, "SourceValueChanged", After)
	end
	return Value
end)
Method("CustomEvent", "SetValue", function(Self, Value)
	if Value == nil then
		error("Argument 1 missing or nil", 0)
	elseif type(Value) == "boolean" then
		Value = if Value then 1 else 0
	else
		Value = Cast.float(Value, 1)
	end
	if Value == Value then
		Value = math.clamp(Value, 0, 1)
	end
	Value = Float32(Value)
	local Before = Extra(Self).Value or 0
	Extra(Self).Value = Value
	if Value ~= Before then
		for Receiver in Receivers(Self) do
			R.FireEvent(Receiver, "SourceValueChanged", Value)
		end
	end
end)
Method("CustomEvent", "GetAttachedReceivers", function(Self)
	local List = {}
	for Receiver, Order in Receivers(Self) do
		table.insert(List, { Receiver, Order })
	end
	table.sort(List, function(A, B)
		return A[2] < B[2]
	end)
	for Index, Entry in List do
		List[Index] = Entry[1]
	end
	return List
end)
Method("CustomEventReceiver", "GetCurrentValue", function(Self)
	local Source = RawGet(Self, "Source")
	return EventValue(if Source and Source:IsA("CustomEvent") then Source else nil)
end)

-- AccessoryDescription / MakeupDescription:GetAppliedInstance: the instance a HumanoidDescription applied to a Humanoid made for
-- it. Not under a description: "AccessoryDescription is not parented to a HumanoidDescription. Did you call ApplyDescription()?";
-- under one that is not under a Humanoid: "HumanoidDescription is not parented to a Humanoid. ..." (a MakeupDescription names
-- itself there); applied, it reports itself (applying a description is not emulated).
for _, Class in { "AccessoryDescription", "MakeupDescription" } do
	Method(Class, "GetAppliedInstance", function(Self)
		local Description = Self.Parent
		if not Description or not Description:IsA("HumanoidDescription") then
			error(`{Class} is not parented to a HumanoidDescription. Did you call ApplyDescription()?`, 0)
		elseif not Description.Parent or not Description.Parent:IsA("Humanoid") then
			error(`{if Class == "MakeupDescription" then Class else "HumanoidDescription"} is not parented to a Humanoid. Did you call ApplyDescription()?`, 0)
		end
		Missing(Class, "GetAppliedInstance")
		return nil
	end)
end
-- GetStyledPropertyChangedSignal(property) is the property's changed signal, as GetPropertyChangedSignal gives it ("Signal
-- AssetIdChanged"; "Nope is not a valid property name."): a style sheet's changes are the GUI layer's
Method("Instance", "GetStyledPropertyChangedSignal", function(Self, Property)
	if Property == nil then
		error("Argument 1 missing or nil", 0)
	end
	return Self:GetPropertyChangedSignal(Property)
end)

-- HumanoidDescription:GetAccessories / SetAccessories (measured likewise): the description keeps one ordered list of
-- accessories. A rigid one comes from its type's property (HatAccessory, HairAccessory, FaceAccessory, NeckAccessory,
-- ShouldersAccessory, FrontAccessory, BackAccessory, WaistAccessory: ids separated by commas, spaces around them dropped, what
-- is no number skipped), and a write of that property takes its type's accessories out and puts the ones it names at the end;
-- a layered one is kept apart from the properties. GetAccessories(includeRigid) answers new tables, the list in order (the
-- layered ones alone without includeRigid): AccessoryType, AssetId, IsLayered, Position (0, 0, 0), Rotation (0, 0, 0),
-- Scale (1, 1, 1) unless given, and a layered one's Order and Puffiness (1 unless given). SetAccessories(list, includeRigid)
-- puts the list's accessories in its order: with includeRigid every accessory (each rigid type's property written with its
-- ids, the others ""), else the layered ones alone, after the rigid ones kept. An entry is dropped unless it is a table with an
-- AccessoryType item (not Unknown; a rigid one of the eight rigid types) and a number AssetId (rounded half away from zero). An
-- entry with a number Order is layered (IsLayered may be left out), one without is rigid, and an IsLayered that says otherwise
-- drops it. A description's list is built from its properties, in the properties' order, when it is first used.
local RigidOrder = { "Hat", "Hair", "Face", "Neck", "Shoulder", "Front", "Back", "Waist" }
local RigidField = { Hat = "HatAccessory", Hair = "HairAccessory", Face = "FaceAccessory", Neck = "NeckAccessory", Shoulder = "ShouldersAccessory", Front = "FrontAccessory", Back = "BackAccessory", Waist = "WaistAccessory" }
local function AccessoryIds(Text)
	local Ids = {}
	for Part in string.gmatch(tostring(Text or ""), "[^,]+") do
		local Id = tonumber((string.gsub(Part, "^%s*(.-)%s*$", "%1")))
		if Id then
			table.insert(Ids, Id)
		end
	end
	return Ids
end
local function TakeRigid(Found, Type, Text)
	local Kept = {}
	for _, Entry in Found.Accessories do
		if Entry.Layered or Entry.Type ~= Type then
			table.insert(Kept, Entry)
		end
	end
	for _, Id in AccessoryIds(Text) do
		table.insert(Kept, { Type = Type, AssetId = Id })
	end
	Found.Accessories = Kept
	Found.Texts[RigidField[Type]] = Text
end
local function Accessories(Self)
	local Found = Extra(Self)
	if not Found.Accessories then
		Found.Accessories, Found.Texts = {}, {}
		for _, Type in RigidOrder do
			TakeRigid(Found, Type, RawGet(Self, RigidField[Type]))
		end
	end
	-- (a property changed by no script write since, by a clone or the engine: its type's accessories read again)
	for _, Type in RigidOrder do
		local Text = RawGet(Self, RigidField[Type])
		if Text ~= Found.Texts[RigidField[Type]] then
			TakeRigid(Found, Type, Text)
		end
	end
	return Found
end
for _, Type in RigidOrder do
	ContentSetter("HumanoidDescription", RigidField[Type], function(Self, Value)
		TakeRigid(Accessories(Self), Type, if type(Value) == "number" then tostring(Value) else Value)
		return Value
	end)
end
local function IncludeRigid(Value, Index)
	if Value == nil then
		error(`Argument {Index} missing or nil`, 0)
	elseif type(Value) == "number" then
		return Value ~= 0
	elseif type(Value) ~= "boolean" then
		error(`Unable to cast {CastName(Value)} to bool`, 0)
	end
	return Value
end
local function AccessoryTable(Entry)
	local Item = if Entry.Layered then Entry.AccessoryType else Enum.AccessoryType[Entry.Type]
	local Out = {
		AccessoryType = Item,
		AssetId = Entry.AssetId,
		IsLayered = Entry.Layered == true,
		Position = Entry.Position or Vector3.zero,
		Rotation = Entry.Rotation or Vector3.zero,
		Scale = Entry.Scale or Vector3.one,
	}
	if Entry.Layered then
		Out.Order = Entry.Order
		Out.Puffiness = Entry.Puffiness or 1
	end
	return Out
end
Method("HumanoidDescription", "GetAccessories", function(Self, Include)
	Include = IncludeRigid(Include, 1)
	local List = {}
	for _, Entry in Accessories(Self).Accessories do
		if Include or Entry.Layered then
			table.insert(List, AccessoryTable(Entry))
		end
	end
	return List
end)
local RigidType = {}
for _, Type in RigidOrder do
	RigidType[Enum.AccessoryType[Type]] = Type
end
local function AccessoryEntry(Given)
	if type(Given) ~= "table" then
		return nil
	end
	local Item, Id = rawget(Given, "AccessoryType"), rawget(Given, "AssetId")
	if typeof(Item) ~= "EnumItem" or Item.EnumType ~= Enum.AccessoryType or Item == Enum.AccessoryType.Unknown or type(Id) ~= "number" or Id ~= Id then
		return nil
	end
	local function Vector(Key)
		local Value = rawget(Given, Key)
		return if typeof(Value) == "Vector3" then Value else nil
	end
	local Entry = { AssetId = Round(Id), Position = Vector("Position"), Rotation = Vector("Rotation"), Scale = Vector("Scale") }
	local Order, Layered = rawget(Given, "Order"), rawget(Given, "IsLayered")
	if Layered ~= nil and Layered ~= (type(Order) == "number") then
		return nil
	end
	if type(Order) == "number" then
		Entry.Layered, Entry.AccessoryType, Entry.Order = true, Item, Order
		local Puffiness = rawget(Given, "Puffiness")
		Entry.Puffiness = if type(Puffiness) == "number" then Puffiness else nil
	else
		if not RigidType[Item] then
			return nil
		end
		Entry.Type = RigidType[Item]
	end
	return Entry
end
Method("HumanoidDescription", "SetAccessories", function(Self, List, Include)
	Include = IncludeRigid(Include, 2)
	Cast.Array(List, 1)
	local Found = Accessories(Self)
	local Kept = {}
	if not Include then
		for _, Entry in Found.Accessories do
			if not Entry.Layered then
				table.insert(Kept, Entry)
			end
		end
	end
	for Index = 1, rawlen(List) do
		local Entry = AccessoryEntry(rawget(List, Index))
		if Entry and (Include or Entry.Layered) then
			table.insert(Kept, Entry)
		end
	end
	Found.Accessories = Kept
	if Include then
		for _, Type in RigidOrder do
			local Ids = {}
			for _, Entry in Kept do
				if not Entry.Layered and Entry.Type == Type then
					table.insert(Ids, tostring(Entry.AssetId))
				end
			end
			local Text = table.concat(Ids, ",")
			Found.Texts[RigidField[Type]] = Text
			RawSet(Self, RigidField[Type], Text)
		end
	end
end)


-- The data stores' and memory stores' synchronous members, a playing client's and a server's alike (Studio 0.747 through roblox-sim,
-- whose stores' network members answer from its own Luau while their constructors and argument casts are the engine's:
-- tools/parity/probes/stores_sync.luau). A store's name and scope are string parameters (StringParam: 5 "5", true "true", a table
-- "{}", a Vector3 "1, 2, 3", an Instance "Instance", an enum item its name, the engine's warning before a value that is no number):
-- no name raises "Argument 1 missing or nil" and no scope is "global", "" raises "DataStore name can't be empty string" ("DataStore
-- scope can't be empty string") and more than 50 bytes "DataStore name is too long" ("DataStore scope is too long"), all without a
-- position; options that are no Instance raise "Unable to cast value to Object", an Instance of another class "510: Options instance
-- not of type DataStoreOptions". A store is made once for its kind, name and scope, a child of DataStoreService named its name
-- (GetGlobalDataStore's is a DataStore named ""). GetRequestBudgetForRequestType casts its type as an enum argument.
-- SetRateLimitForRequestType casts from the last argument to the first ("Argument 3 missing or nil" with fewer than three, "Unable to
-- cast string to int"), then refuses UpdateAsync and OnUpdate ("SetRateLimitForRequestType: Does not apply to
-- DataStoreRequestType.UpdateAsync.") and a base limit out of its range ("SetRateLimitForRequestType: proposed base limit 61 for
-- DataStoreRequestType.GetAsync is out of range [0, 60]."); the ranges are Roblox's documentation's, the per-player limit's text is
-- taken as the base limit's (not measured), and the limit set is the budget GetRequestBudgetForRequestType answers from then on (the
-- documentation: it overrides the default). MemoryStoreService's GetSortedMap, GetHashMap and GetQueue cast their arguments (no name
-- "Argument 1 missing or nil", a timeout that is no number "Unable to cast string to int") and then refuse a client: "MemoryStoreService
-- must be called from a Server." (a server's structures are server.luau's); GetDistributedCounter raises "MemoryStoreDistributedCounter
-- is not enabled." on both sides once its name is cast. The stores' network members and MessagingService's report themselves not
-- emulated on a client: roblox-sim answers them from its own Luau, so what a client raises is not measured.
-- (a store's or a memory structure's name: server.luau's memory stores read theirs the same way)
local function StoreText(Value, Argument)
	return StringParam.Read(Value, Argument)
end
R.StoreText = StoreText
R.CastAll = CastAll
local function StoreName(Value, Label, Default, Argument)
	if Value == nil then
		if Default then
			return Default
		end
		error(`Argument {Argument} missing or nil`, 0)
	end
	local Text = StoreText(Value, Argument)
	if #Text == 0 then
		error(`DataStore {Label} can't be empty string`, 0)
	elseif #Text > 50 then
		error(`DataStore {Label} is too long`, 0)
	end
	return Text
end
local StoreInstances = {}
-- (made once for its kind, name and scope; a server's data for it is server.luau's: R.StoreMade)
local function StoreInstance(Kind, Class, Name, Scope)
	local Id = `{Kind}\0{Name}\0{Scope}`
	local Made = StoreInstances[Id]
	if not Made then
		Made = R.Create(Class)
		R.RawSet(Made, "Name", Name, true)
		R.SetParent(Made, game:GetService("DataStoreService"))
		StoreInstances[Id] = Made
		if R.StoreMade then
			R.StoreMade(Made, Kind, Name, Scope)
		end
	end
	return Made
end
Method("DataStoreService", "GetDataStore", function(_, Name, Scope, Options)
	if Options ~= nil and typeof(Options) ~= "Instance" then
		error("Unable to cast value to Object", 0)
	end
	Scope = StoreName(Scope, "scope", "global", 2)
	Name = StoreName(Name, "name", nil, 1)
	if Options ~= nil and not Options:IsA("DataStoreOptions") then
		error("510: Options instance not of type DataStoreOptions", 0)
	end
	if Options ~= nil and Options.AllScopes then
		R.Missing("DataStoreOptions", "AllScopes")
	end
	return StoreInstance("Standard", "DataStore", Name, Scope)
end)
Method("DataStoreService", "GetGlobalDataStore", function()
	return StoreInstance("Global", "DataStore", "", "global")
end)
Method("DataStoreService", "GetOrderedDataStore", function(_, Name, Scope)
	Scope = StoreName(Scope, "scope", "global", 2)
	Name = StoreName(Name, "name", nil, 1)
	return StoreInstance("Ordered", "OrderedDataStore", Name, Scope)
end)
-- The budget a minute as Roblox's documentation gives it ({base, per player}): the legacy request types' table and the Standard /
-- Ordered ones'; SetRateLimitForRequestType's limits in their place once set. Requests are never throttled and spend nothing.
local Budgets = {
	GetAsync = { 60, 10 },
	SetIncrementAsync = { 60, 10 },
	UpdateAsync = { 60, 10 },
	GetSortedAsync = { 5, 2 },
	SetIncrementSortedAsync = { 30, 5 },
	OnUpdate = { 30, 5 },
	ListAsync = { 5, 2 },
	GetVersionAsync = { 5, 2 },
	RemoveVersionAsync = { 5, 2 },
	StandardRead = { 60, 40 },
	StandardWrite = { 60, 40 },
	StandardList = { 5, 2 },
	StandardRemove = { 60, 40 },
	OrderedRead = { 60, 40 },
	OrderedWrite = { 30, 5 },
	OrderedList = { 5, 2 },
	OrderedRemove = { 30, 5 },
}
-- (the most SetRateLimitForRequestType takes: Roblox's documentation; UpdateAsync and OnUpdate take none)
local RateRanges = {
	GetAsync = { 60, 40 },
	SetIncrementAsync = { 60, 40 },
	GetSortedAsync = { 5, 2 },
	SetIncrementSortedAsync = { 30, 5 },
	ListAsync = { 5, 2 },
	GetVersionAsync = { 5, 2 },
	RemoveVersionAsync = { 5, 2 },
	StandardRead = { 10000, 200 },
	StandardWrite = { 10000, 200 },
	StandardList = { 10000, 200 },
	StandardRemove = { 10000, 200 },
	OrderedRead = { 10000, 200 },
	OrderedWrite = { 10000, 200 },
	OrderedList = { 10000, 200 },
	OrderedRemove = { 10000, 200 },
}
local RateLimits = {}
Method("DataStoreService", "GetRequestBudgetForRequestType", function(_, ...)
	local Type = CastAll({ { "DataStoreRequestType" } }, ...)
	local Budget = RateLimits[Type.Name] or Budgets[Type.Name] or { 60, 10 }
	return Budget[1] + Budget[2] * #game:GetService("Players"):GetPlayers()
end)
Method("DataStoreService", "SetRateLimitForRequestType", function(_, ...)
	local Type, Base, PerPlayer = CastAll({ { "DataStoreRequestType" }, { "int" }, { "int" } }, ...)
	local Range = RateRanges[Type.Name]
	if not Range then
		error(`SetRateLimitForRequestType: Does not apply to DataStoreRequestType.{Type.Name}.`, 0)
	elseif Base < 0 or Base > Range[1] then
		error(`SetRateLimitForRequestType: proposed base limit {Base} for DataStoreRequestType.{Type.Name} is out of range [0, {Range[1]}].`, 0)
	elseif PerPlayer < 0 or PerPlayer > Range[2] then
		error(`SetRateLimitForRequestType: proposed per player limit {PerPlayer} for DataStoreRequestType.{Type.Name} is out of range [0, {Range[2]}].`, 0)
	end
	RateLimits[Type.Name] = { Base, PerPlayer }
end)
for _, Name in { "GetSortedMap", "GetHashMap", "GetQueue" } do
	Method("MemoryStoreService", Name, function(_, Structure, Timeout)
		if Name == "GetQueue" then
			Cast.int(Timeout, 2, 30)
		end
		StoreText(Structure, 1)
		error("MemoryStoreService must be called from a Server.", 0)
	end)
end
Method("MemoryStoreService", "GetDistributedCounter", function(_, Name)
	StoreText(Name, 1)
	error("MemoryStoreDistributedCounter is not enabled.", 0)
end)

-- Player's friends (measured in a 0.747 playing client and server: tools/parity/probes/loopback/player_friends.luau and
-- player_friends_refused.luau, captured five and three times, alike). IsFriendsWith and IsFriendsWithAsync(userId), GetFriendsOnline and
-- GetFriendsOnlineAsync(maxFriends = 200), GetFriendsInServerAsync() and GetFriendsInUniverseAsync() (which take no argument)
-- check self first, then cast their argument (FriendUser, FriendCount). A count over 200 warns "GetFriendsOnline only returns
-- a maximum of 200 friends", on a server too, and no more than 200 come back (Roblox's documentation). GetFriendsOnline and
-- GetFriendsInUniverseAsync are the local player's alone: on a server, or for another player, they raise after the cast and
-- the warning, and only then is a negative count refused. A negative user id answers false at once. Everything else is a
-- question the player asks the web (FriendQuestion), its friends (IsFriendsWith by either name, GetFriendsInServerAsync) or its
-- online friends (the other three). What the web answers is the profile's (profile.luau); the offline player has no friends
-- (false, empty lists), where the sim's client, signed in to no account, gets "HTTP 401".
local FriendsOnlineCap = 200
local function FriendCastName(Value)
	return if type(Value) == "thread" then "null" else CastName(Value)
end
-- A "User" argument (a user id): a number cut toward zero, NaN, the infinities and numbers out of int64's range -2^63; true 1,
-- false 0; a string only a whole decimal with an optional "-" and spaces around it (R.Int64Argument's text)
local function FriendUser(Value)
	local Kind = type(Value)
	if Value == nil then
		error("Argument 1 missing or nil", 0)
	elseif Kind == "boolean" then
		return if Value then 1 else 0
	elseif Kind == "number" then
		if Value ~= Value or Value >= 2 ^ 63 or Value < -2 ^ 63 then
			return -2 ^ 63
		end
		return if Value < 0 then math.ceil(Value) else math.floor(Value)
	elseif Kind == "string" then
		local Id = R.Int64Argument(Value, 1)
		if Id then
			return tonumber(Id)
		end
	end
	error(`Unable to cast {FriendCastName(Value)} to User`, 0)
end
-- An "int" count: nil the default, a number rounded half away from zero, NaN, the infinities and numbers out of int's range
-- -2^31; true 1, false 0; a string only a whole decimal in int's range
local function FriendCount(Value)
	local Kind = type(Value)
	if Value == nil then
		return FriendsOnlineCap
	elseif Kind == "boolean" then
		return if Value then 1 else 0
	elseif Kind == "number" then
		local Whole = if Value < 0 then -math.floor(-Value + 0.5) else math.floor(Value + 0.5)
		return if Whole ~= Whole or Whole < -2 ^ 31 or Whole >= 2 ^ 31 then -2 ^ 31 else Whole
	elseif Kind == "string" then
		local Count = tonumber((R.Int64Argument(Value, 1)))
		if Count and Count >= -2 ^ 31 and Count < 2 ^ 31 then
			return Count
		end
	end
	error(`Unable to cast {FriendCastName(Value)} to int`, 0)
end
-- A question the player asks the web about its friends (Kind "friends") or its online friends ("online"). Its first one of a
-- kind waits for the answer, here a resumption; where the thread cannot yield it raises "thread is not yieldable", the question
-- still asked, so a question of that kind in the same frame waits and one in a later frame has its answer. Later questions
-- answer at once: the online friends for 30 s (Roblox's documentation: GetFriendsOnlineAsync's cache), the friends for good
-- (its documentation: "results are cached"). Each player asks its own (a client asked again for another player's friends).
local function FriendQuestion(Self, Kind)
	local Data = Extra(Self)
	local Asked = Data.FriendQuestions
	if not Asked then
		Asked = {}
		Data.FriendQuestions = Asked
	end
	local Now = time()
	local Question = Asked[Kind]
	if Question == nil or (Kind == "online" and Now - Question.At >= 30) then
		Question = { At = Now, Waiting = true }
		Asked[Kind] = Question
	end
	if Question.Waiting then
		if Question.At == Now then
			if not coroutine.isyieldable() then
				error("thread is not yieldable", 0)
			end
			task.wait()
		end
		Question.Waiting = false
	end
end
local function FriendSelf(Self, Name)
	if typeof(Self) ~= "Instance" or not Self:IsA("Player") then
		error(`Expected ':' not '.' calling member function {Name}`, 0)
	end
end
for _, Name in { "IsFriendsWith", "IsFriendsWithAsync" } do
	Method("Player", Name, function(Self, UserId)
		FriendSelf(Self, Name)
		if FriendUser(UserId) < 0 then
			return false
		end
		FriendQuestion(Self, "friends")
		return false
	end)
end
for _, Name in { "GetFriendsOnline", "GetFriendsOnlineAsync" } do
	Method("Player", Name, function(Self, MaxFriends)
		FriendSelf(Self, Name)
		local Count = FriendCount(MaxFriends)
		if Count > FriendsOnlineCap then
			R.LogLine("warn", `GetFriendsOnline only returns a maximum of {FriendsOnlineCap} friends`, nil)
		end
		if Self ~= R.LocalPlayer() then
			error("GetFriendsOnline must be invoked on the Local Player", 0)
		elseif Count < 0 then
			error("GetFriendsOnline maxFriends argument can not be negative", 0)
		end
		FriendQuestion(Self, "online")
		return {}
	end)
end
Method("Player", "GetFriendsInServerAsync", function(Self)
	FriendSelf(Self, "GetFriendsInServerAsync")
	FriendQuestion(Self, "friends")
	return {}
end)
Method("Player", "GetFriendsInUniverseAsync", function(Self)
	FriendSelf(Self, "GetFriendsInUniverseAsync")
	if Self ~= R.LocalPlayer() then
		error("GetFriendsInUniverse must be invoked on the Local Player", 0)
	end
	FriendQuestion(Self, "online")
	return {}
end)
-- (profile.luau reads the cast arguments as these do)
R.FriendUser = FriendUser
R.FriendCount = function(Value)
	return math.min(FriendCount(Value), FriendsOnlineCap)
end

-- Decal.ColorMap is its Texture under another name (derived.rs): a write drops an EditableImage Texture or TextureContent shows, and it
-- reads "" while one is shown, as Texture does (measured, roblox-sim 0.747: decal_color_map.luau)
ContentSetter("Decal", "ColorMap", function(Self, Value) Unshow(Self, "Texture") return Value end)
R.ContentGetter("Decal", "ColorMap", function(Self, Value) return if ShownImage(Self, "Texture") then "" else Value end)
-- A clone of an instance one of whose properties shows an EditableImage shows the image too, written or destroyed since, and reads ""
-- for its string id (measured, roblox-sim 0.747: an ImageLabel, an ImageButton inside a Frame, a Decal, a MeshPart and a
-- SurfaceAppearance; image_clone.luau); its later writes reach the copy as the template's do
R.OnImageClone(function(Original, Copy)
	local Held = Extras[Original]
	if not (Held and Held.Shown) or next(Held.Shown) == nil then
		return
	end
	local Shown = {}
	for Key, Image in Held.Shown do
		Shown[Key] = Image
		local Data = Extra(Image)
		Data.Users = Data.Users or setmetatable({}, { __mode = "k" })
		Data.Users[Copy] = Data.Users[Copy] or {}
		Data.Users[Copy][Key] = true
	end
	Extra(Copy).Shown = Shown
	R.ShowsImage(Copy)
end)

-- GuiService:GetInsetArea (InsetArea, declared above) in a playing client's ScreenGui coordinates, where y is measured from the top
-- bar's inset (GetGuiInset): with Roblox's top bar (device.topbarInset > 0, a playing client) None and DeviceSafeInsets are the whole
-- screen (0, -inset, width, height - inset), CoreUISafeInsets the screen below the bar (0, 0, width, height - inset), and
-- TopbarSafeInsets the bar's band right of Roblox's buttons (TopbarInset.Min.X, -inset, width, 0); a client without a top bar answers
-- the whole screen (0, 0, width, height) for each. The argument is an item, its name or its value; another value raises "Unable to cast
-- string to ScreenInsets", none "Argument 1 missing or nil" (measured, roblox-sim 0.747 with its core scripts, rendered 1280 x 720
-- and 844 x 390 touch clients, and a client without them: tools/parity/probes/topbar_inset.luau).
InsetArea = function(Insets)
	local Name = CastEnum("ScreenInsets", Insets, 1).Name
	local Inset, Width, Height = Options.TopbarInset, Options.ViewportWidth, Options.ViewportHeight
	if Inset <= 0 then
		return Rect.new(0, 0, Width, Height)
	elseif Name == "TopbarSafeInsets" then
		return Rect.new(R.TopbarStart(), -Inset, Width, 0)
	end
	return Rect.new(0, if Name == "CoreUISafeInsets" then 0 else -Inset, Width, Height - Inset)
end

-- Player.User: what a client signed in to no account reads for every player, its own and the others' (typeof User, its text below,
-- Id 0, any other key "X is not a valid member of User"; each read a value of its own, equal to every other: measured, roblox-sim
-- 0.747, a client and a server, tools/parity/probes/player_members.luau, loopback/player_user.luau). A signed-in client's is that
-- account's, not measured.
local function UserIndex(_, Key)
	if Key == "Id" then
		return 0
	end
	error(`{tostring(Key)} is not a valid member of User`, 0)
end
local function UserText()
	return "U1.AQAAAAAAAAAAAAAAAAAAAAA"
end
local function UserEqual()
	return true
end
Getter("Player", "User", function()
	local User = newproxy(true)
	local Meta = getmetatable(User)
	Meta.__index = UserIndex
	Meta.__tostring = UserText
	Meta.__eq = UserEqual
	Meta.__metatable = "The metatable is locked"
	R.Register(User, "User")
	return User
end)

-- CollectionService:CreateCollection(query, root): a CollectionHandle over a QueryDescendants query below root (the Workspace when
-- root is nil or left out), whose members services/collections.rs keeps. The handle reads typeof CollectionHandle and tostring
-- Collection; Query and Root are read-only ("Collection.Query is read-only and cannot be written to"), GetCount and Destroy its
-- methods (on another self "invalid argument #1 to 'GetCount' (CollectionHandle expected, got Instance)" at the caller, none
-- "missing argument #1 ..."), any other key "X is not a valid member of CollectionHandle", a key that is no string Luau's "invalid
-- argument #2 (string expected, got number)". A function may be assigned once to OnAdded, OnRemoved, OnChanged, OnItemAdded or
-- OnItemRemoved ("Callback OnAdded already assigned"); another value raises "Expected function, got number", another name that
-- starts with On "OnFoo cannot be assigned to", any other "Foo is not a valid signal name, must start with 'On'", and each reads
-- as no member. After Destroy (or its root's) every member raises "Collection does not exist". The root is cast first ("Unable to
-- cast value to Object"), then the query ("Argument 1 missing or nil"; a number reads as its text), and a malformed query raises
-- QueryDescendants' words (measured, roblox-sim 0.747, a client and a server: tools/parity/probes/collection_handles.luau).
local CollectionIds = setmetatable({}, { __mode = "k" })
local Collections = {}
local CollectionCallbacks = { OnAdded = true, OnRemoved = true, OnChanged = true, OnItemAdded = true, OnItemRemoved = true }
-- (R's collection functions answer a value, or nil and the error, raised here as the engine raises: no position)
local function Answer(Value, Error)
	if Error ~= nil then
		error(Error, 0)
	end
	return Value
end
local function CollectionSelf(Name, ...)
	local Id = CollectionIds[(...)]
	if Id then
		return Id
	elseif select("#", ...) == 0 then
		error(`missing argument #1 to '{Name}' (CollectionHandle expected)`, 3)
	end
	error(`invalid argument #1 to '{Name}' (CollectionHandle expected, got {typeof((...))})`, 3)
end
local CollectionMethods = {
	GetCount = function(...)
		return Answer(R.CollectionCount(CollectionSelf("GetCount", ...)))
	end,
	Destroy = function(...)
		Answer(R.CollectionDestroy(CollectionSelf("Destroy", ...)))
	end,
}
local function NewCollection(Id)
	local Handle = newproxy(true)
	local Meta = getmetatable(Handle)
	local Assigned = {}
	Meta.__type = "CollectionHandle"
	Meta.__index = function(_, Key)
		if type(Key) ~= "string" then
			error(`invalid argument #2 (string expected, got {typeof(Key)})`, 2)
		elseif Key == "Query" then
			return Answer(R.CollectionQuery(Id))
		elseif Key == "Root" then
			return Answer(R.CollectionRoot(Id))
		end
		local Found = CollectionMethods[Key]
		if Found == nil then
			error(`{Key} is not a valid member of CollectionHandle`, 0)
		end
		return Found
	end
	Meta.__newindex = function(_, Key, Value)
		if Key == "Query" or Key == "Root" then
			error(`Collection.{Key} is read-only and cannot be written to`, 0)
		elseif type(Value) ~= "function" then
			error(`Expected function, got {typeof(Value)}`, 0)
		elseif type(Key) ~= "string" or string.sub(Key, 1, 2) ~= "On" then
			error(`{tostring(Key)} is not a valid signal name, must start with 'On'`, 0)
		elseif not CollectionCallbacks[Key] then
			error(`{Key} cannot be assigned to`, 0)
		elseif Assigned[Key] then
			error(`Callback {Key} already assigned`, 0)
		end
		Answer(R.CollectionCallback(Id, Key, Value))
		Assigned[Key] = true
	end
	Meta.__tostring = function()
		return "Collection"
	end
	Meta.__metatable = "The metatable is locked"
	R.Register(Handle, "CollectionHandle")
	CollectionIds[Handle] = Id
	Collections[Id] = Handle
	return Handle
end
Method("CollectionService", "CreateCollection", function(_, Query, Root)
	if Root == nil then
		Root = workspace
	elseif typeof(Root) ~= "Instance" then
		error("Unable to cast value to Object", 0)
	end
	return NewCollection(Answer(R.CollectionCreate(StringParam.Read(Query, 1), Root)))
end)
R.CollectionHandles = function(Instance)
	local Found = {}
	for _, Id in Answer(R.CollectionsOf(Instance)) do
		table.insert(Found, Collections[Id])
	end
	return Found
end

return true
