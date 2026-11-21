-- Datatype misuse: what a wrong operand, argument or self, an unknown member, an assignment and an operator a type
-- lacks raise (or answer), for every datatype scripts construct.
local function Show(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		return P.num(Value)
	elseif Kind == "string" then
		return '"' .. Value .. '"'
	elseif Kind == "nil" or Kind == "boolean" then
		return tostring(Value)
	elseif Kind == "table" then
		return "table"
	end
	return Kind .. "(" .. (string.gsub(tostring(Value), "0x%x+", "ADDR")) .. ")"
end
local function Try(Function, ...)
	local Results = table.pack(pcall(Function, ...))
	if Results[1] then
		local Parts = {}
		for Index = 2, Results.n do
			table.insert(Parts, Show(Results[Index]))
		end
		return "ok " .. table.concat(Parts, ", ")
	end
	local Message = Results[2]
	if type(Message) ~= "string" then
		return "error (" .. typeof(Message) .. ") " .. (string.gsub(string.split(tostring(Message), "\n")[1], "0x%x+", "ADDR"))
	end
	return "error " .. P.err(Message)
end

local Folder = Instance.new("Folder")
local Bads = {
	{ "nil", nil },
	{ "number", 1 },
	{ "string", "x" },
	{ "numstr", "2" },
	{ "table", {} },
	{ "bool", true },
	{ "Vector3", Vector3.one },
	{ "Vector2", Vector2.one },
	{ "CFrame", CFrame.identity },
	{ "Instance", Folder },
}

local Types = {
	{ "Vector2", function() return Vector2.new(1, 2) end, "X", { "Abs", "Ceil", "Floor", "Sign", "Angle", "Cross", "Dot", "FuzzyEq", "Lerp", "Max", "Min" } },
	{ "Vector3", function() return Vector3.new(1, 2, 3) end, "X", { "Abs", "Ceil", "Floor", "Sign", "Angle", "Cross", "Dot", "FuzzyEq", "Lerp", "Max", "Min" } },
	{ "CFrame", function() return CFrame.new(1, 2, 3) end, "X", { "Inverse", "Lerp", "Orthonormalize", "ToWorldSpace", "ToObjectSpace", "PointToWorldSpace", "PointToObjectSpace", "VectorToWorldSpace", "VectorToObjectSpace", "GetComponents", "ToEulerAnglesXYZ", "ToEulerAnglesYXZ", "ToOrientation", "ToAxisAngle", "FuzzyEq", "AngleBetween", "ToEulerAngles" } },
	{ "UDim", function() return UDim.new(1, 2) end, "Scale", {} },
	{ "UDim2", function() return UDim2.new(1, 2, 3, 4) end, "X", { "Lerp" } },
	{ "Vector3int16", function() return Vector3int16.new(1, 2, 3) end, "X", {} },
	{ "Vector2int16", function() return Vector2int16.new(1, 2) end, "X", {} },
	{ "Rect", function() return Rect.new(1, 2, 3, 4) end, "Width", {} },
	{ "Color3", function() return Color3.new(0.25, 0.5, 0.75) end, "R", { "Lerp", "ToHSV", "ToHex" } },
	{ "NumberRange", function() return NumberRange.new(1, 2) end, "Min", {} },
	{ "BrickColor", function() return BrickColor.new("Bright red") end, "Name", {} },
	{ "Region3", function() return Region3.new(Vector3.new(0, 0, 0), Vector3.new(4, 4, 4)) end, "Size", { "ExpandToGrid" } },
	{ "Region3int16", function() return Region3int16.new(Vector3int16.new(0, 0, 0), Vector3int16.new(1, 1, 1)) end, "Min", {} },
	{ "Ray", function() return Ray.new(Vector3.zero, Vector3.one) end, "Origin", { "ClosestPoint", "Distance" } },
	{ "TweenInfo", function() return TweenInfo.new(1) end, "Time", {} },
	{ "DateTime", function() return DateTime.fromUnixTimestamp(0) end, "UnixTimestamp", { "ToUniversalTime", "ToLocalTime", "ToIsoDate", "FormatUniversalTime", "FormatLocalTime" } },
	{ "NumberSequence", function() return NumberSequence.new(1) end, "Keypoints", {} },
	{ "ColorSequence", function() return ColorSequence.new(Color3.new()) end, "Keypoints", {} },
	{ "NumberSequenceKeypoint", function() return NumberSequenceKeypoint.new(0, 1) end, "Time", {} },
	{ "ColorSequenceKeypoint", function() return ColorSequenceKeypoint.new(0, Color3.new()) end, "Time", {} },
	{ "PhysicalProperties", function() return PhysicalProperties.new(1, 1, 1) end, "Density", {} },
	{ "Axes", function() return Axes.new(Enum.Axis.X) end, "X", {} },
	{ "Faces", function() return Faces.new(Enum.NormalId.Top) end, "Top", {} },
	{ "Font", function() return Font.new("rbxasset://fonts/families/Arimo.json") end, "Family", {} },
	{ "Random", function() return Random.new(1) end, "Nope", {} },
	{ "Content", function() return Content.fromUri("rbxassetid://1") end, "Uri", {} },
}

local Binary = {
	{ "add", function(A, B) return A + B end },
	{ "sub", function(A, B) return A - B end },
	{ "mul", function(A, B) return A * B end },
	{ "div", function(A, B) return A / B end },
	{ "idiv", function(A, B) return A // B end },
	{ "mod", function(A, B) return A % B end },
	{ "pow", function(A, B) return A ^ B end },
	{ "lt", function(A, B) return A < B end },
	{ "le", function(A, B) return A <= B end },
	{ "eq", function(A, B) return A == B end },
	{ "concat", function(A, B) return A .. B end },
}

for _, Entry in Types do
	local Name, Make, Member, Methods = Entry[1], Entry[2], Entry[3], Entry[4]
	local V = Make()
	-- operators: the value with itself, then with each other kind on its right and on its left
	print(`unary {Name} | unm {Try(function() return -V end)} | len {Try(function() return #V end)} | call {Try(function() return V() end)} | iter {Try(function() for _ in V do end end)}`)
	for _, Op in Binary do
		local Parts = { `op {Name} {Op[1]} | self {Try(Op[2], V, Make())}` }
		for _, Bad in Bads do
			table.insert(Parts, `{Bad[1]} {Try(Op[2], V, Bad[2])}`)
		end
		table.insert(Parts, "| rev")
		for _, Bad in Bads do
			table.insert(Parts, `{Bad[1]} {Try(Op[2], Bad[2], V)}`)
		end
		print(table.concat(Parts, " | "))
	end
	-- members
	print(`member {Name} | Nope {Try(function() return V.Nope end)} | nope {Try(function() return V.nope end)} | 1 {Try(function() return V[1] end)} | true {Try(function() return V[true] end)} | lower {Try(function() return V[string.lower(Member)] end)} | method {Try(function() return V:Nope() end)}`)
	print(`assign {Name} | {Member} {Try(function() V[Member] = 1 end)} | Nope {Try(function() V.Nope = 1 end)} | 1 {Try(function() V[1] = 1 end)} | method {Try(function() V[Methods[1] or "Nope"] = 1 end)}`)
	-- methods: no argument, each kind as argument #2 (by `:` and by `.`), each kind as self
	for _, Method in Methods do
		local Colon = { `method {Name}:{Method} | none {Try(function() return V[Method](V) end)}` }
		local Dot = { `method {Name}.{Method} | none {Try(function() return V[Method]() end)}` }
		local Self = { `method {Name} self {Method}` }
		for _, Bad in Bads do
			local Function = V[Method]
			table.insert(Colon, `{Bad[1]} {Try(function() return Function(V, Bad[2]) end)}`)
			table.insert(Dot, `{Bad[1]} {Try(function() return Function(V, V, Bad[2]) end)}`)
			table.insert(Self, `{Bad[1]} {Try(function() return Function(Bad[2], V) end)}`)
		end
		print(table.concat(Colon, " | "))
		print(table.concat(Dot, " | "))
		print(table.concat(Self, " | "))
	end
end

-- methods by namecall: a wrong argument and a wrong self through `:` itself (the call form a client names no method in)
local function Namecall(Name, Value, Method, Arg)
	return Try(function()
		if Method == "Dot" then return Value:Dot(Arg) elseif Method == "Lerp" then return Value:Lerp(Arg, 0.5) elseif Method == "Lerp2" then return Value:Lerp(Value, Arg)
		elseif Method == "FuzzyEq" then return Value:FuzzyEq(Arg) elseif Method == "Cross" then return Value:Cross(Arg) elseif Method == "Max" then return Value:Max(Arg)
		elseif Method == "Angle" then return Value:Angle(Arg) elseif Method == "ToWorldSpace" then return Value:ToWorldSpace(Arg) elseif Method == "PointToObjectSpace" then return Value:PointToObjectSpace(Arg)
		elseif Method == "ToHex" then return Value:ToHex(Arg) elseif Method == "ExpandToGrid" then return Value:ExpandToGrid(Arg) elseif Method == "ClosestPoint" then return Value:ClosestPoint(Arg)
		elseif Method == "FormatUniversalTime" then return Value:FormatUniversalTime(Arg, "en-us") end
		return nil
	end)
end
for _, Case in {
	{ "Vector2", Vector2.new(1, 2), { "Dot", "Lerp", "Lerp2", "FuzzyEq", "Cross", "Max", "Angle" } },
	{ "Vector3", Vector3.new(1, 2, 3), { "Dot", "Lerp", "Lerp2", "FuzzyEq", "Cross", "Max", "Angle" } },
	{ "CFrame", CFrame.new(1, 2, 3), { "Lerp", "Lerp2", "ToWorldSpace", "PointToObjectSpace" } },
	{ "UDim2", UDim2.new(1, 2, 3, 4), { "Lerp", "Lerp2" } },
	{ "Color3", Color3.new(0.25, 0.5, 0.75), { "Lerp", "Lerp2", "ToHex" } },
	{ "Region3", Region3.new(Vector3.zero, Vector3.one * 4), { "ExpandToGrid" } },
	{ "Ray", Ray.new(Vector3.zero, Vector3.one), { "ClosestPoint" } },
	{ "DateTime", DateTime.fromUnixTimestamp(0), { "FormatUniversalTime" } },
} do
	for _, Method in Case[3] do
		local Parts = { `namecall {Case[1]}:{Method}` }
		for _, Bad in Bads do
			table.insert(Parts, `{Bad[1]} {Namecall(Case[1], Case[2], Method, Bad[2])}`)
		end
		print(table.concat(Parts, " | "))
	end
end

-- constructors
local V2, V3 = Vector2.one, Vector3.one
local Constructors = {
	{ "Vector2.new", Vector2.new }, { "Vector3.new", Vector3.new }, { "Vector3.FromNormalId", Vector3.FromNormalId }, { "Vector3.FromAxis", Vector3.FromAxis },
	{ "CFrame.new", CFrame.new }, { "CFrame.Angles", CFrame.Angles }, { "CFrame.fromEulerAnglesXYZ", CFrame.fromEulerAnglesXYZ }, { "CFrame.fromEulerAnglesYXZ", CFrame.fromEulerAnglesYXZ },
	{ "CFrame.fromOrientation", CFrame.fromOrientation }, { "CFrame.lookAt", CFrame.lookAt }, { "CFrame.lookAlong", CFrame.lookAlong }, { "CFrame.fromMatrix", CFrame.fromMatrix },
	{ "CFrame.fromAxisAngle", CFrame.fromAxisAngle }, { "CFrame.fromEulerAngles", CFrame.fromEulerAngles },
	{ "UDim.new", UDim.new }, { "UDim2.new", UDim2.new }, { "UDim2.fromScale", UDim2.fromScale }, { "UDim2.fromOffset", UDim2.fromOffset },
	{ "Vector3int16.new", Vector3int16.new }, { "Vector2int16.new", Vector2int16.new }, { "Rect.new", Rect.new },
	{ "Color3.new", Color3.new }, { "Color3.fromRGB", Color3.fromRGB }, { "Color3.fromHSV", Color3.fromHSV }, { "Color3.fromHex", Color3.fromHex },
	{ "NumberRange.new", NumberRange.new }, { "BrickColor.new", BrickColor.new }, { "BrickColor.palette", BrickColor.palette },
	{ "Region3.new", Region3.new }, { "Region3int16.new", Region3int16.new }, { "Ray.new", Ray.new }, { "TweenInfo.new", TweenInfo.new },
	{ "DateTime.fromUnixTimestamp", DateTime.fromUnixTimestamp }, { "DateTime.fromIsoDate", DateTime.fromIsoDate }, { "DateTime.fromUniversalTime", DateTime.fromUniversalTime },
	{ "NumberSequence.new", NumberSequence.new }, { "ColorSequence.new", ColorSequence.new }, { "NumberSequenceKeypoint.new", NumberSequenceKeypoint.new },
	{ "ColorSequenceKeypoint.new", ColorSequenceKeypoint.new }, { "PhysicalProperties.new", PhysicalProperties.new }, { "Axes.new", Axes.new }, { "Faces.new", Faces.new },
	{ "Font.new", Font.new }, { "Font.fromName", Font.fromName }, { "Font.fromId", Font.fromId }, { "Random.new", Random.new },
}
local Args = {
	{ "none", {} },
	{ "x", { "x" } },
	{ "table", { {} } },
	{ "bool", { true } },
	{ "V2", { V2 } },
	{ "V3", { V3 } },
	{ "1,x", { 1, "x" } },
	{ "1,2,x", { 1, 2, "x" } },
	{ "1,2,3,x", { 1, 2, 3, "x" } },
	{ "V3,x", { V3, "x" } },
	{ "V3,V3,x", { V3, V3, "x" } },
	{ "nil,1", { nil, 1 } },
}
for _, Constructor in Constructors do
	local Parts = { `new {Constructor[1]}` }
	for _, Arg in Args do
		local Values = Arg[2]
		local Count = if Arg[1] == "nil,1" then 2 else #Values
		table.insert(Parts, `{Arg[1]} {Try(function() return Constructor[2](table.unpack(Values, 1, Count)) end)}`)
	end
	print(table.concat(Parts, " | "))
end
-- a constructor through `:` (self as the first argument) and by a direct call line (positions)
print(`new-line | {Try(function() return Vector2.new("x") end)} | {Try(function() return CFrame.new("x") end)} | {Try(function() return UDim.new("x") end)} | {Try(function() return Rect.new("x") end)} | {Try(function() return Vector3int16.new("x") end)} | {Try(function() return UDim2.new("x") end)}`)

-- library tables
for _, Name in { "Vector2", "CFrame", "UDim", "Rect", "Color3", "Vector3int16" } do
	local Library = getfenv()[Name]
	print(`library {Name} | Nope {Try(function() return Library.Nope end)} | assign {Try(function() Library.Nope = 1 end)} | assign-new {Try(function() Library.new = 1 end)}`)
end

-- `:` calls a client names no method in, argument counts, and what a reflected float takes
local A2, A3, ACF, AR, ADT = Vector2.new(1, 2), Vector3.new(1, 2, 3), CFrame.new(1, 2, 3), Ray.new(Vector3.zero, Vector3.one), DateTime.fromUnixTimestamp(0)
print(`colon Vector2 | Max {Try(function() return A2:Max(nil) end)} | Min {Try(function() return A2:Min(A2, 5) end)} | Angle {Try(function() return A2:Angle() end)} | Angle4 {Try(function() return A2:Angle(A2, true, 1) end)} | Angle3 {Try(function() return A2:Angle(A2, 5) end)} | FuzzyEq {Try(function() return A2:FuzzyEq() end)} | FuzzyEq4 {Try(function() return A2:FuzzyEq(A2, 0.01, 1) end)} | Abs {Try(function() return A2:Abs(1) end)}`)
print(`colon Vector3 | Dot {Try(function() return A3:Dot() end)} | Cross {Try(function() return A3:Cross(1) end)} | Lerp {Try(function() return A3:Lerp(A3) end)} | Lerp3 {Try(function() return A3:Lerp(A3, true) end)} | Lerp-str {Try(function() return A3:Lerp(A3 * 3, "0.5") end)} | Max {Try(function() return A3:Max(A3, 5) end)} | FuzzyEq {Try(function() return A3:FuzzyEq() end)} | FuzzyEq3 {Try(function() return A3:FuzzyEq(A3, "x") end)} | Angle {Try(function() return A3:Angle() end)} | Angle4 {Try(function() return A3:Angle(A3, A3, 1) end)} | Angle3 {Try(function() return A3:Angle(A3, 5) end)}`)
print(`colon CFrame | ToWorldSpace {Try(function() return ACF:ToWorldSpace(nil) end)} | ToWorldSpace2 {Try(function() return ACF:ToWorldSpace(ACF, 5) end)} | PointToWorldSpace {Try(function() return ACF:PointToWorldSpace(nil) end)} | ToEulerAngles {Try(function() return ACF:ToEulerAngles(nil) end)} | ToEulerAngles-num {Try(function() return ACF:ToEulerAngles(1) end)} | ToEulerAngles-enum {Try(function() return ACF:ToEulerAngles(Enum.KeyCode.A) end)} | Lerp {Try(function() return ACF:Lerp() end)} | Inverse {Try(function() return ACF:Inverse(5) end)}`)
print(`colon Ray | ClosestPoint {Try(function() return AR:ClosestPoint() end)} | Distance {Try(function() return AR:Distance(5) end)} | Distance2 {Try(function() return AR:Distance(Vector3.one, 5) end)}`)
print(`colon DateTime | Format {Try(function() return ADT:FormatUniversalTime() end)} | Format1 {Try(function() return ADT:FormatUniversalTime("x") end)} | Format-bad {Try(function() return ADT:FormatUniversalTime("x", 5) end)} | Format-table {Try(function() return ADT:FormatUniversalTime("x", {}) end)} | Iso {Try(function() return ADT:ToIsoDate(5) end)}`)
print(`self-string DateTime | {Try(function() return DateTime.fromUnixTimestamp(0).ToIsoDate("2020-01-01T00:00:00Z") end)} | {Try(function() return DateTime.fromUnixTimestamp(0).ToUniversalTime("86400").Day end)} | {Try(function() return DateTime.fromUnixTimestamp(0).ToIsoDate("") end)}`)
print(`self Region3 | {Try(function() return Region3.new(Vector3.zero, Vector3.one).ExpandToGrid(1, 4) end)} | {Try(function() return Region3.new(Vector3.zero, Vector3.one).ExpandToGrid(nil, 4) end)} | {Try(function() return Region3.new(Vector3.zero, Vector3.one * 4).ExpandToGrid(Region3.new(Vector3.zero, Vector3.one * 4), "2") end)}`)
for _, Case in {
	{ "Vector2", Vector2.new(0, 0), Vector2.new(10, 10) },
	{ "CFrame", CFrame.new(0, 0, 0), CFrame.new(10, 10, 10) },
	{ "UDim2", UDim2.new(0, 0, 0, 0), UDim2.new(10, 10, 10, 10) },
	{ "Color3", Color3.new(0, 0, 0), Color3.new(1, 1, 1) },
} do
	local From, To = Case[2], Case[3]
	print(`lerp-alpha {Case[1]} | true {Try(function() return From:Lerp(To, true) end)} | false {Try(function() return From:Lerp(To, false) end)} | numstr {Try(function() return From:Lerp(To, "0.25") end)} | hex {Try(function() return From:Lerp(To, "0x1") end)} | spaced {Try(function() return From:Lerp(To, " 0.5 ") end)}`)
end

-- Vector3int16 and Vector2int16 arithmetic: fractions, zero, overflow, and what an operand of another type reads as
for _, Case in {
	{ "Vector3int16", Vector3int16.new(10, -10, 7), Vector3int16.new(0, 3, -2), function(V) return V.X .. " " .. V.Y .. " " .. V.Z end },
	{ "Vector2int16", Vector2int16.new(10, -10), Vector2int16.new(0, 3), function(V) return V.X .. " " .. V.Y end },
} do
	local V, W, Text = Case[2], Case[3], Case[4]
	local function Op(Function)
		local Ok, Result = pcall(Function)
		if Ok then
			return if typeof(Result) == Case[1] then Text(Result) else Show(Result)
		end
		return "error " .. P.err(Result)
	end
	print(`int16 {Case[1]} mul | 1.5 {Op(function() return V * 1.5 end)} | -1.5 {Op(function() return V * -1.5 end)} | 0.5 {Op(function() return V * 0.5 end)} | "1.5" {Op(function() return V * "1.5" end)} | 4000 {Op(function() return V * 4000 end)} | 1e10 {Op(function() return V * 1e10 end)} | nan {Op(function() return V * (0 / 0) end)} | W {Op(function() return V * W end)} | rev2.5 {Op(function() return 2.5 * V end)} | revnil {Op(function() return nil * V end)}`)
	print(`int16 {Case[1]} div | 2 {Op(function() return V / 2 end)} | 3 {Op(function() return V / 3 end)} | -3 {Op(function() return V / -3 end)} | 0.5 {Op(function() return V / 0.5 end)} | 0 {Op(function() return V / 0 end)} | "x" {Op(function() return V / "x" end)} | W {Op(function() return V / W end)} | rev100 {Op(function() return 100 / V end)} | rev0 {Op(function() return 0 / W end)} | rev1.5 {Op(function() return 1.5 / V end)} | revW {Op(function() return W / V end)}`)
	print(`int16 {Case[1]} add | over {Op(function() return V + V * 3276 end)} | unm {Op(function() return -(V * 3276 - V * 3276 - V * 3277) end)}`)
end

-- assignment and calls of members of other kinds
print(`assign-more | V2.x {Try(function() A2.x = 1 end)} | V2[true] {Try(function() A2[true] = 1 end)} | V3.x {Try(function() A3.x = 1 end)} | V3[true] {Try(function() A3[true] = 1 end)} | CF.p {Try(function() ACF.p = 1 end)} | CF.FuzzyEq {Try(function() ACF.FuzzyEq = 1 end)} | UDim.scale {Try(function() UDim.new().scale = 1 end)} | UDim[true] {Try(function() UDim.new()[true] = 1 end)} | V2.Magnitude {Try(function() A2.Magnitude = 1 end)}`)
print(`call-property | V2:X {Try(function() return A2:X() end)} | V3:X {Try(function() return A3:X() end)} | CF:Position {Try(function() return ACF:Position() end)} | UDim:Scale {Try(function() return UDim.new():Scale() end)} | Rect:Width {Try(function() return Rect.new():Width() end)} | DT:UnixTimestamp {Try(function() return ADT:UnixTimestamp() end)}`)
print(`member-more | V3[nil] {Try(function() return A3[nil :: any] end)} | V2[A2] {Try(function() return A2[A2] end)} | UDim[A2] {Try(function() return UDim.new()[A2] end)} | UDim[1.5] {Try(function() return UDim.new()[1.5] end)} | TweenInfo[1] {Try(function() return TweenInfo.new()[1] end)} | DT[1] {Try(function() return ADT[1] end)}`)

-- constructors: more forms
local B3 = Vector3.one
print(`new-more CFrame | fromMatrix2 {Try(function() return CFrame.fromMatrix(B3, B3) end)} | fromMatrix4 {Try(function() return CFrame.fromMatrix(B3, B3, B3, "x") end)} | fromEuler1 {Try(function() return CFrame.fromEulerAngles(B3, 2, 3) end)} | fromEuler2 {Try(function() return CFrame.fromEulerAngles(1, B3, 3) end)} | fromEuler-nil {Try(function() return CFrame.fromEulerAngles(1, 2, 3, nil) end)} | fromEuler-num {Try(function() return CFrame.fromEulerAngles(1, 2, 3, 1) end)} | lookAt-nil {Try(function() return CFrame.lookAt(B3, B3 * 2, nil) end)} | new7 {Try(function() return CFrame.new(1, 2, 3, 0, 0, 0, "x") end)} | new12 {Try(function() return CFrame.new(1, 2, 3, 1, 0, 0, 0, 1, 0, 0, 0, "x") end)} | new3nil {Try(function() return CFrame.new(1, nil, 3) end)} | new5 {Try(function() return CFrame.new(1, 2, 3, 4, 5) end)} | new-str {Try(function() return CFrame.new("1", "2", "3") end)} | Angles-str {Try(function() return CFrame.Angles("1", true, false) end)} | axisangle-v2 {Try(function() return CFrame.fromAxisAngle(Vector2.one, 1) end)}`)
print(`new-more Rect | swapped {Try(function() return Rect.new(3, 4, 1, 2) end)} | nil4 {Try(function() return Rect.new(1, 2, 3, nil) end)} | str4 {Try(function() return Rect.new("1", "2", "3", "4") end)} | v2x {Try(function() return Rect.new(Vector2.one, "x") end)} | v2v2 {Try(function() return Rect.new(Vector2.one, Vector2.zero) end)} | 5 {Try(function() return Rect.new(1, 2, 3, 4, 5) end)}`)
print(`new-more Color3 | hsv-str {Try(function() return Color3.fromHSV("0.5", "1", "1") end)} | hsv4 {Try(function() return Color3.fromHSV(0.5, 1, 1, 1) end)} | hex-num {Try(function() return Color3.fromHex(255) end)} | hex2 {Try(function() return Color3.fromHex("#fff", 1) end)} | rgb-str {Try(function() return Color3.fromRGB("255", "x", 1) end)} | new-str {Try(function() return Color3.new("0.5", "x", true) end)}`)

-- rules the fixes filled in: enum casts of numbers and names, three Vector3s to CFrame.new, palette's range, Random's ==
print(`gaps FromNormalId | 7 {Try(Vector3.FromNormalId, 7)} | Top {Try(Vector3.FromNormalId, "Top")} | 1.5 {Try(Vector3.FromNormalId, 1.5)} | KeyCode {Try(Vector3.FromNormalId, Enum.KeyCode.A)} | axis-Y {Try(Vector3.FromAxis, "Y")} | axis-1 {Try(Vector3.FromAxis, -1)}`)
print(`gaps CFrame.new | 3v {Try(function() return CFrame.new(Vector3.new(1, 2, 3), Vector3.new(4, 5, 7), Vector3.new(0, 1, 0)) end)} | 2v-1 {Try(function() return CFrame.new(Vector3.new(1, 2, 3), Vector3.new(4, 5, 7), 1) end)}`)
print(`gaps palette | true {Try(BrickColor.palette, true)} | "2" {Try(BrickColor.palette, "2")} | 200 {Try(BrickColor.palette, 200)} | -1 {Try(BrickColor.palette, -1)}`)
print(`gaps Random | 1-2 {tostring(Random.new(1) == Random.new(2))} | 1-1 {tostring(Random.new(1) == Random.new(1))} | stepped {Try(function() local A = Random.new(1) A:NextNumber() return A == Random.new(1) end)} | clone {Try(function() local A = Random.new(1) A:NextNumber() return A == A:Clone() end)}`)
