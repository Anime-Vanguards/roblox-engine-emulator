-- HttpService:JSONEncode / JSONDecode: numbers, strings, arrays against dictionaries, errors, and generated round trips.
local Http = game:GetService("HttpService")
local function Encode(Value)
	local Ok, Text = pcall(Http.JSONEncode, Http, Value)
	return if Ok then Text else "E(" .. P.err(Text) .. ")"
end
local function Decode(Text)
	local Ok, Value = pcall(Http.JSONDecode, Http, Text)
	if not Ok then
		return "E(" .. P.err(Value) .. ")"
	end
	-- re-encode what came back to show it; a dictionary's keys in sorted order so hash order does not matter
	local function Show(Item)
		local Kind = type(Item)
		if Kind == "table" then
			local Keys = {}
			for Key in Item do
				table.insert(Keys, Key)
			end
			table.sort(Keys, function(A, B)
				return tostring(A) < tostring(B)
			end)
			local Parts = {}
			for _, Key in Keys do
				table.insert(Parts, (if type(Key) == "number" then "#" .. P.num(Key) else Key) .. ":" .. Show(Item[Key]))
			end
			return "{" .. table.concat(Parts, ",") .. "}"
		elseif Kind == "number" then
			return "n" .. P.num(Item)
		elseif Kind == "string" then
			return string.format("%q", Item)
		end
		return tostring(Item)
	end
	return Show(Value)
end

-- numbers
local Numbers = { 0, -0, 1, -1, 0.5, 0.1, 1 / 3, 100, 1e15, 1e16, 1e17, 1e20, 1e21, 2 ^ 53, 2 ^ 53 + 2, 2 ^ 63, 1e300, 1e-7, 5e-324, 123456789.125, 0.1 + 0.2, -1e-10, 4294967296, math.huge, -math.huge, 0 / 0 }
local Out = {}
for _, Value in Numbers do
	table.insert(Out, Encode(Value))
end
print("encode-numbers " .. table.concat(Out, " "))
Out = {}
for _, Value in Numbers do
	table.insert(Out, Encode({ Value }))
end
print("encode-number-arrays " .. table.concat(Out, " "))

-- strings and escapes
local Strings = { "", "plain", "quote\"", "back\\slash", "slash/", "new\nline", "tab\t", "cr\r", "nul\0byte", "\1\2\31", "\127", "é", "€", "😀", "\255", "\192\128", "<>&'", "\u{2028}\u{2029}" }
Out = {}
for _, Value in Strings do
	table.insert(Out, Encode(Value))
end
print("encode-strings " .. table.concat(Out, " "))

-- tables
print("encode-empty " .. Encode({}))
print("encode-array " .. Encode({ 1, "two", true, false }))
print("encode-nested " .. Encode({ { 1, { 2, { 3 } } } }))
print("encode-dict-one " .. Encode({ Key = 1 }))
print("encode-mixed " .. Encode({ 1, 2, Key = 3 }))
print("encode-sparse " .. Encode({ [1] = "a", [3] = "c" }))
print("encode-hole " .. Encode({ 1, nil, 3 }))
print("encode-zero-key " .. Encode({ [0] = "z" }))
print("encode-float-key " .. Encode({ [1.5] = "x" }))
print("encode-bool-key " .. Encode({ [true] = "x" }))
print("encode-numeric-string-key " .. Encode({ ["1"] = "x" }))
print("encode-negative-key " .. Encode({ [-1] = "x" }))
print("encode-function " .. Encode({ print }))
print("encode-instance " .. Encode({ workspace }))
print("encode-vector3 " .. Encode({ Vector3.new(1, 2, 3) }))
print("encode-enum " .. Encode({ Enum.Material.Plastic }))
print("encode-nil " .. Encode(nil))
print("encode-bool " .. Encode(true))
print("encode-buffer " .. Encode(buffer.create(2)))
local Cyclic = {}
Cyclic[1] = Cyclic
print("encode-cyclic " .. Encode(Cyclic))
local Deep = {}
local Cursor = Deep
for _ = 1, 600 do
	Cursor[1] = {}
	Cursor = Cursor[1]
end
print("encode-deep " .. string.sub(Encode(Deep), 1, 60))
print("encode-metatable " .. Encode(setmetatable({ 1 }, { __index = function()
	return 2
end })))
local Shared = { 1 }
print("encode-shared " .. Encode({ Shared, Shared }))

-- decoding
local Texts = { "1", "-0", "1.0", "1e2", "1E+2", "0.1", "1e400", "-1e400", "123456789012345678901234567890", "9007199254740993", "\"a\\u00e9\"", "\"\\ud83d\\ude00\"", "\"\\ud83d\"", "\"\\/\"", "\"tab\tin\"", "[1,2,3]", "[]", "{}", "{\"a\":1,\"b\":[true,false,null]}", "[null,1]", "[1,null]", "{\"a\":null}", "{\"1\":\"x\"}", "{\"1\":\"x\",\"2\":\"y\"}", " [ 1 , 2 ] ", "[1,]", "{\"a\":1,}", "01", "+1", ".5", "NaN", "Infinity", "'x'", "", "nul", "true false", "\"unterminated", "[1 2]", "{\"a\" 1}", "\"\\x\"", "{\"a\":1,\"a\":2}", "\"\\u0000\"" }
for _, Text in Texts do
	print("decode " .. string.format("%q", Text) .. " -> " .. Decode(Text))
end
print("decode-nonstring " .. P.try(Http.JSONDecode, Http, 5))

-- generated round trips
local Rng = P.rng(777)
local function Make(Depth)
	local Kind = Rng.int(1, if Depth > 2 then 4 else 6)
	if Kind == 1 then
		return Rng.int(-1000, 1000) / Rng.pick({ 1, 2, 3, 10, 7 })
	elseif Kind == 2 then
		local Chars = {}
		for _ = 1, Rng.int(0, 6) do
			table.insert(Chars, string.char(Rng.pick({ 97, 98, 34, 92, 10, 233, 32, 48, 0, 127, 1 })))
		end
		return table.concat(Chars)
	elseif Kind == 3 then
		return Rng.int(0, 1) == 1
	elseif Kind == 4 then
		return Rng.int(-2 ^ 31, 2 ^ 31)
	elseif Kind == 5 then
		local List = {}
		for Index = 1, Rng.int(0, 4) do
			List[Index] = Make(Depth + 1)
		end
		return List
	end
	local Map = {}
	for _ = 1, Rng.int(1, 4) do
		Map["k" .. Rng.int(1, 9)] = Make(Depth + 1)
	end
	return Map
end
for Case = 1, 40 do
	local Value = Make(0)
	local Text = Encode(Value)
	print("roundtrip " .. Case .. " " .. Decode(Text))
end
print("generateguid " .. P.try(function()
	local Guid = Http:GenerateGUID(false)
	return #Guid .. " " .. tostring(string.match(Guid, "^%x+%-%x+%-%x+%-%x+%-%x+$") ~= nil) .. " " .. #Http:GenerateGUID(true) .. " " .. #Http:GenerateGUID()
end))
print("urlencode " .. P.try(Http.UrlEncode, Http, "a b&c=d/é?~*'()!") )
