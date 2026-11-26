-- Datatype edge rules taken until now from the forum, rbxmk or reasoning: the sequence constructors' checks and their
-- order, a keypoint's envelope, NumberRange's order, BrickColor's nearest colour on a tie, where Random's wide
-- NextInteger starts, a part's Size below zero and past its bound, DateTime's range when time units roll and
-- fromIsoDate's trailing text and hours, Euler angles at gimbal lock, lookAt with another up vector, fromMatrix's
-- missing Z column, getmetatable's lock texts, Font.fromEnum and Content's refusals, an int64 argument's spaces,
-- a params object's enum lists in its text, and a clone's attribute naming its own tree.
local function CF(Value)
	return P.f32s(Value:GetComponents())
end
local function V3(Value)
	return P.f32s(Value.X, Value.Y, Value.Z)
end

-- sequences
local function NK(Time, Value, Envelope)
	if Envelope == nil then
		return NumberSequenceKeypoint.new(Time, Value)
	end
	return NumberSequenceKeypoint.new(Time, Value, Envelope)
end
local function CK(Time)
	return ColorSequenceKeypoint.new(Time, Color3.new(0.25, 0.5, 0.75))
end
local function Times(Sequence)
	local Parts = {}
	for _, Point in Sequence.Keypoints do
		table.insert(Parts, P.f32(Point.Time))
	end
	return table.concat(Parts, ",")
end
local SequenceCases = {
	{ "ordered", { 0, 0.5, 1 } },
	{ "unordered", { 0, 0.6, 0.4, 1 } },
	{ "equal-times", { 0, 0.5, 0.5, 1 } },
	{ "end-short", { 0, 0.9 } },
	{ "start-late", { 0.1, 1 } },
	{ "both-ends", { 0.1, 0.9 } },
	{ "unordered-end-short", { 0, 0.6, 0.4, 0.9 } },
	{ "unordered-start-late", { 0.1, 0.6, 0.4, 1 } },
	{ "one", { 0 } },
	{ "one-at-1", { 1 } },
	{ "two-at-0", { 0, 0 } },
	{ "ends-near", { 0.00005, 0.99995 } },
	{ "ends-near-2", { 0.0001, 0.9999 } },
	{ "ends-off", { 0.0002, 0.9998 } },
	{ "ends-negative", { -0.00005, 1.00005 } },
	{ "ends-negative-off", { -0.001, 1 } },
	{ "beyond", { 0, 1, 1.5 } },
	{ "twenty", { 0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9, 1 } },
	{ "twenty-one", { 0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9, 0.95, 1 } },
}
for _, Case in SequenceCases do
	print("numbersequence " .. Case[1] .. " " .. P.try(function()
		local Points = {}
		for _, Time in Case[2] do
			table.insert(Points, NK(Time, 1))
		end
		return Times(NumberSequence.new(Points))
	end))
	print("colorsequence " .. Case[1] .. " " .. P.try(function()
		local Points = {}
		for _, Time in Case[2] do
			table.insert(Points, ColorSequenceKeypoint.new(Time, Color3.new(0.5, 0.5, 0.5)))
		end
		return Times(ColorSequence.new(Points))
	end))
end
print("numbersequence-element " .. P.try(NumberSequence.new, { NK(0, 1), 5, NK(1, 1) }) .. " | " .. P.try(NumberSequence.new, { NK(0, 1), CK(0.5), NK(1, 1) }))
print("colorsequence-element " .. P.try(ColorSequence.new, { CK(0), 5, CK(1) }) .. " | " .. P.try(ColorSequence.new, { CK(0), NK(0.5, 1), CK(1) }))
print("numbersequence-nontable " .. P.try(NumberSequence.new, "x") .. " | " .. P.try(NumberSequence.new, true) .. " | " .. P.try(NumberSequence.new, nil))
print("colorsequence-nontable " .. P.try(ColorSequence.new, "x") .. " | " .. P.try(ColorSequence.new, true) .. " | " .. P.try(ColorSequence.new, 5) .. " | " .. P.try(ColorSequence.new, nil))
print("colorsequence-empty " .. P.try(ColorSequence.new, {}) .. " | " .. P.try(NumberSequence.new, {}))

-- keypoints and ranges
for _, Envelope in { -1, -0.0001, 0, 0.5, 2, 0 / 0 } do
	print("keypoint-envelope " .. P.num(Envelope) .. " " .. P.try(function()
		local Point = NK(0.5, 1, Envelope)
		return P.f32(Point.Envelope)
	end))
end
print("keypoint-time " .. P.try(function()
	return P.f32(NK(2, 1).Time) .. " " .. P.f32(NK(-1, 1).Time)
end) .. " | " .. P.try(function()
	return P.f32(CK(2).Time)
end))
for _, Pair in { { 2, 1 }, { 1, 1 }, { 1, 1 - 1e-9 }, { 0 / 0, 1 }, { 1, 0 / 0 }, { math.huge, -math.huge }, { -0.5, -1 } } do
	print("numberrange " .. P.nums(Pair[1], Pair[2]) .. " " .. P.try(function()
		local Range = NumberRange.new(Pair[1], Pair[2])
		return P.f32(Range.Min) .. " " .. P.f32(Range.Max)
	end))
end

-- BrickColor: the colour halfway between two palette colours, which both are as near to (each line one palette colour
-- against every later one)
local Palette = {}
for Index = 0, 127 do
	Palette[Index] = BrickColor.palette(Index)
end
for First = 0, 126 do
	local Answers = {}
	local A = Palette[First].Color
	for Second = First + 1, 127 do
		local B = Palette[Second].Color
		local Middle = Color3.new((A.R + B.R) / 2, (A.G + B.G) / 2, (A.B + B.B) / 2)
		table.insert(Answers, tostring(BrickColor.new(Middle).Number))
	end
	print("brickcolor-tie " .. Palette[First].Number .. " " .. table.concat(Answers, ","))
end
-- the same through a part's Color, read as its BrickColor (the first eight palette colours against every later one)
local Viewer = Instance.new("Part")
for First = 0, 7 do
	local Answers = {}
	local A = Palette[First].Color
	for Second = First + 1, 127 do
		local B = Palette[Second].Color
		Viewer.Color = Color3.new((A.R + B.R) / 2, (A.G + B.G) / 2, (A.B + B.B) / 2)
		table.insert(Answers, tostring(Viewer.BrickColor.Number))
	end
	print("brickcolor-view " .. Palette[First].Number .. " " .. table.concat(Answers, ","))
end
print("brickcolor-rgb " .. BrickColor.new(0.5, 0.5, 0.5).Number .. " " .. BrickColor.new(0, 0, 0).Number .. " " .. BrickColor.new(1, 1, 1).Number .. " " .. BrickColor.new(0.6, 0.4, 0.2).Number)

-- Random: wide NextInteger ranges
for _, Power in { 31, 32, 33, 34, 35, 36, 38, 40, 41, 42, 48, 52, 53 } do
	local Parts = {}
	for _, Range in { { 0, 2 ^ Power - 1 }, { 0, 2 ^ Power }, { 1, 2 ^ Power + 1 }, { -2 ^ (Power - 1), 2 ^ (Power - 1) - 1 }, { -2 ^ (Power - 1), 2 ^ (Power - 1) } } do
		local Generator = Random.new(Power * 7 + 3)
		local Draws = {}
		for _ = 1, 3 do
			table.insert(Draws, P.num(Generator:NextInteger(Range[1], Range[2])))
		end
		table.insert(Parts, table.concat(Draws, ","))
	end
	print("nextinteger 2^" .. Power .. " " .. table.concat(Parts, " | "))
end

-- a part's Size
local Part = Instance.new("Part")
for _, Size in { Vector3.new(-1, 2, 3), Vector3.new(-0.0005, -5, 0), Vector3.new(2048, 2049, 4096), Vector3.new(5000, 1e10, 2047.9),
	Vector3.new(0 / 0, math.huge, -math.huge), Vector3.new(1e-9, 0.0005, 0.002) } do
	print("size " .. V3(Size) .. " " .. P.try(function()
		Part.Size = Size
		return V3(Part.Size)
	end))
end
local Ball = Instance.new("Part")
Ball.Shape = Enum.PartType.Ball
Ball.Size = Vector3.new(-3, 5, 7)
print("size-ball " .. V3(Ball.Size))

-- DateTime
for _, Millis in { -17987443200000, -17987443200001, 253402300799999, 253402300800000, 0 / 0, math.huge, -math.huge, 2 ^ 63 } do
	print("millis-range " .. P.num(Millis) .. " " .. P.try(function()
		return P.num(DateTime.fromUnixTimestampMillis(Millis).UnixTimestampMillis)
	end))
end
for _, Fields in { { 1400, 1, 1 }, { 1400, 1, 1, -1 }, { 1400, 1, 1, 0, -1 }, { 1400, 1, 1, 0, 0, -1 }, { 1400, 1, 1, 0, 0, 0, -1 }, { 1399, 12, 31, 23, 59, 59 },
	{ 1399, 12, 31, 24 }, { 9999, 12, 31, 23, 59, 59 }, { 9999, 12, 31, 24 }, { 9999, 12, 31, 23, 59, 60 }, { 1400, 1, 1, -100000 }, { 2000, 1, 1, 0, 0, 0, -86400000 * 220000 } } do
	print("rolled " .. table.concat(Fields, ",") .. " " .. P.try(function()
		return P.num(DateTime.fromUniversalTime(table.unpack(Fields)).UnixTimestampMillis)
	end))
end
for _, Text in { "2020-02-29T12:34:56Zjunk", "2020-02-29T12:34:56Z ", "2020-02-29T12:34:56.5Zx", "2020-02-29junk", "2020-02-29T12:34:56+02:00x",
	"2020-02-29T12:34:56.789", "2020-02-29T25:00:00Z", "2020-02-29T24:30:00Z", "2020-02-29T24:00:01Z", "2020-02-29T99:00:00Z", "2020-02-29T23:60:00Z",
	"2020-02-29T23:59:61Z", "1399-12-31T23:59:59Z", "1400-01-01T00:00:00Z", "1400-01-01T00:00:00+01:00", "9999-12-31T23:59:59Z", "9999-12-31T24:00:00Z",
	"9999-12-31T23:59:59-01:00", "2020-02-29T12:34:56+24:00", "2020-02-29T12:34:56+99:99" } do
	local Ok, Time = pcall(DateTime.fromIsoDate, Text)
	print("iso " .. Text .. " " .. (if not Ok then "E(" .. P.err(Time) .. ")" elseif Time == nil then "nil" else P.num(Time.UnixTimestampMillis)))
end

-- Euler angles at gimbal lock and a matrix past it
local Half = math.pi / 2
for _, Case in {
	{ "x-pi/2", CFrame.fromEulerAnglesXYZ(0.3, Half, 0.2) },
	{ "x-minus-pi/2", CFrame.fromEulerAnglesXYZ(0.3, -Half, 0.2) },
	{ "yxz-pi/2", CFrame.fromEulerAnglesYXZ(Half, 0.3, 0.2) },
	{ "yxz-minus-pi/2", CFrame.fromEulerAnglesYXZ(-Half, 0.3, 0.2) },
	{ "exact-xyz", CFrame.new(0, 0, 0, 0, 0, 1, 0, 1, 0, -1, 0, 0) },
	{ "exact-yxz", CFrame.new(0, 0, 0, 1, 0, 0, 0, 0, -1, 0, 1, 0) },
	{ "past-one", CFrame.new(0, 0, 0, 0, 0, 1.0000001, 0, 1, 0, -1, 0, 0) },
	{ "past-one-yxz", CFrame.new(0, 0, 0, 1, 0, 0, 0, 0, -1.0000001, 0, 1, 0) },
	{ "identity", CFrame.new() },
} do
	local C = Case[2]
	local Axis, Angle = C:ToAxisAngle()
	print("euler " .. Case[1] .. " xyz " .. P.f32s(C:ToEulerAnglesXYZ()) .. " | yxz " .. P.f32s(C:ToEulerAnglesYXZ()) .. " | orientation " .. P.f32s(C:ToOrientation())
		.. " | zxy " .. P.f32s(C:ToEulerAngles(Enum.RotationOrder.ZXY)) .. " | axis " .. V3(Axis) .. " " .. P.f32(Angle))
end

-- lookAt with another up vector, and fromMatrix without its Z column
local Eye = Vector3.new(1, 2, 3)
for _, Case in {
	{ "up-x", Vector3.new(4, 6, 8), Vector3.xAxis },
	{ "up-z", Vector3.new(4, 6, 8), Vector3.zAxis },
	{ "up-odd", Vector3.new(4, 6, 8), Vector3.new(1, 1, 0) },
	{ "up-long", Vector3.new(4, 6, 8), Vector3.new(0, 10, 0) },
	{ "along-up-x", Eye + Vector3.xAxis, Vector3.xAxis },
	{ "along-minus-up-x", Eye - Vector3.xAxis, Vector3.xAxis },
	{ "near-up-x", Eye + Vector3.new(1, 0.0001, 0), Vector3.xAxis },
	{ "up-zero", Vector3.new(4, 6, 8), Vector3.zero },
} do
	print("lookat " .. Case[1] .. " " .. P.try(function()
		return CF(CFrame.lookAt(Eye, Case[2], Case[3]))
	end))
end
for _, Case in {
	{ "unit", Vector3.xAxis, Vector3.yAxis },
	{ "skew", Vector3.new(1, 0.5, 0), Vector3.new(0, 1, 0.25) },
	{ "long", Vector3.new(2, 0, 0), Vector3.new(0, 3, 0) },
	{ "parallel", Vector3.xAxis, Vector3.xAxis * 2 },
	{ "zero", Vector3.zero, Vector3.yAxis },
} do
	print("frommatrix " .. Case[1] .. " " .. P.try(function()
		return CF(CFrame.fromMatrix(Eye, Case[2], Case[3]))
	end) .. " | with-z " .. P.try(function()
		return CF(CFrame.fromMatrix(Eye, Case[2], Case[3], Vector3.new(0, 0, 2)))
	end))
end

-- metatables' lock texts
for _, Case in { { "string", "" }, { "instance", workspace }, { "vector3", Vector3.new() }, { "cframe", CFrame.new() }, { "enum", Enum.Material },
	{ "enumitem", Enum.Material.Wood }, { "enums", Enum }, { "signal", workspace.Changed }, { "color3", Color3.new() } } do
	local Meta = getmetatable(Case[2])
	print("metatable " .. Case[1] .. " " .. type(Meta) .. " " .. (if type(Meta) == "string" then Meta else "-"))
end

-- Font and Content refusals
print("font-unknown " .. P.try(function()
	return Font.fromEnum(Enum.Font.Unknown).Family
end))
for _, Id in { 0, -1, 1.5, 0 / 0, math.huge, -math.huge, 2 ^ 53 } do
	print("content-assetid " .. P.num(Id) .. " " .. P.try(function()
		return tostring(Content.fromAssetId(Id).Uri)
	end))
end
print("content-object " .. P.try(function()
	return Content.fromObject(nil)
end) .. " | " .. P.try(function()
	return Content.fromObject(5)
end))

-- an int64 argument's spaces (a group id; the first call naming a group waits for the web)
local Player = game:GetService("Players").LocalPlayer
if Player then
	for Index, Text in { " 5", "5 ", "\t6", "6\t", "\n7", "\v8", "\f9", "\r10", "11\n", "\t 12 \t", "1 3", "0x10" } do
		print("int64 " .. Index .. " " .. P.try(function()
			return Player:GetRankInGroup(Text)
		end))
	end
else
	print("int64 no player")
end

-- a params object's enum lists in its text
local Search = CatalogSearchParams.new()
print("catalog-none " .. tostring(Search))
Search.AssetTypes = { Enum.AvatarAssetType.Hat }
print("catalog-one " .. tostring(Search))
Search.AssetTypes = { Enum.AvatarAssetType.Hat, Enum.AvatarAssetType.Shirt, Enum.AvatarAssetType.Pants }
Search.BundleTypes = { Enum.BundleType.BodyParts, Enum.BundleType.Animations }
print("catalog-many " .. tostring(Search))

-- a clone's attribute naming its own tree
local Model = Instance.new("Model")
Model.Name = "Original"
local Inside = Instance.new("Part")
Inside.Name = "Inside"
Inside.Parent = Model
Model:SetAttribute("Self", Model)
Model:SetAttribute("Child", Inside)
Model:SetAttribute("Outside", workspace)
local Copy = Model:Clone()
Copy.Name = "Copy"
for _, Name in { "Self", "Child", "Outside" } do
	local Handle = Copy:GetAttribute(Name)
	local Target = Handle and Handle:Get()
	print("clone-attribute " .. Name .. " " .. typeof(Handle) .. " " .. tostring(Handle) .. " copy " .. tostring(Target == Copy or Target == Copy:FindFirstChild("Inside"))
		.. " original " .. tostring(Target == Model or Target == Inside))
end
