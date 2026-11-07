-- HttpService:JSONEncode and JSONDecode cases json.luau leaves open: the other control escapes, every kind of value
-- inside a dictionary, every kind of key, a dictionary's key order, where nesting stops, and more of the decoder's grammar.
local Http = game:GetService("HttpService")
local function Encode(Value)
	local Ok, Text = pcall(Http.JSONEncode, Http, Value)
	return (string.gsub(if Ok then Text else "E(" .. P.err(Text) .. ")", "0x%x+", "ADDR"))
end
local function Decode(Text)
	local Ok, Value = pcall(Http.JSONDecode, Http, Text)
	if not Ok then
		return "E(" .. P.err(Value) .. ")"
	end
	if type(Value) == "number" then
		return "n" .. P.num(Value)
	elseif type(Value) == "table" then
		local Count = 0
		for _ in Value do
			Count += 1
		end
		return "table#" .. #Value .. "/" .. Count
	end
	return type(Value) .. ":" .. P.text(tostring(Value))
end

print("escapes " .. Encode("\b\f\v\0\27\127\u{80}\u{FFFF}\u{10FFFF}"))
local Values = { { "function", print }, { "instance", workspace }, { "vector3", Vector3.new(1, 2, 3) }, { "enum", Enum.Material.Plastic },
	{ "buffer", buffer.create(1) }, { "thread", coroutine.create(print) }, { "color3", Color3.new(1, 0, 0) }, { "inf", math.huge },
	{ "nan", 0 / 0 }, { "cframe", CFrame.new() }, { "udim2", UDim2.new() }, { "empty", {} } }
local Parts = {}
for _, Pair in Values do
	table.insert(Parts, Pair[1] .. "=" .. Encode({ a = Pair[2] }))
end
print("dict-values " .. table.concat(Parts, " "))
Parts = {}
for _, Pair in Values do
	table.insert(Parts, Pair[1] .. "=" .. Encode({ 1, Pair[2], 3 }))
end
print("array-values " .. table.concat(Parts, " "))
local Keys = { { "instance", workspace }, { "vector3", Vector3.new(1, 2, 3) }, { "function", print }, { "table", {} }, { "enum", Enum.Material.Plastic },
	{ "huge", 1e20 }, { "negative-half", -0.5 }, { "negative-zero", -0 }, { "color3", Color3.new(1, 0, 0) }, { "buffer", buffer.create(1) } }
Parts = {}
for _, Pair in Keys do
	table.insert(Parts, Pair[1] .. "=" .. Encode({ [Pair[2]] = 1 }))
end
print("keys " .. table.concat(Parts, " "))
print("key-order " .. Encode({ zeta = 1, alpha = 2, mid = 3, beta = 4, Alpha = 5, ["10"] = 6, ["9"] = 7, [""] = 8 }))
print("key-order-again " .. Encode({ beta = 4, ["9"] = 7, Alpha = 5, alpha = 2, [""] = 8, mid = 3, ["10"] = 6, zeta = 1 }))
print("trailing-nil " .. Encode({ 1, 2, nil }) .. " " .. Encode({ nil, 2 }) .. " " .. Encode({ [2] = 2 }))
local Limits = {}
for _, Depth in { 50, 100, 150, 199, 200, 201, 255, 256, 257, 300, 500, 1000 } do
	local Nest = {}
	local Cursor = Nest
	for _ = 2, Depth do
		Cursor[1] = {}
		Cursor = Cursor[1]
	end
	local Ok, Text = pcall(Http.JSONEncode, Http, Nest)
	table.insert(Limits, Depth .. "=" .. (if Ok then "ok" .. #Text else "E(" .. P.err(Text) .. ")"))
end
print("depth " .. table.concat(Limits, " "))
local DecodeLimits = {}
for _, Depth in { 50, 100, 199, 200, 201, 256, 257, 500, 1000, 5000 } do
	table.insert(DecodeLimits, Depth .. "=" .. Decode(string.rep("[", Depth) .. string.rep("]", Depth)))
end
print("decode-depth " .. table.concat(DecodeLimits, " "))
Parts = {}
for _, Text in { "1e-400", "-1e-400", "5.", "1.e5", "0x10", " ", "\"\\u00\"", "\"\\uD83D\\uDE00\"", "\"\255\"", "18446744073709551616", "-9223372036854775809",
	"9223372036854775807", "[1,2,3]\n", "\"\\b\\f\\n\\r\\t\"", "true", "null", "\"a\"", "[[]]", "{\"a\":{\"b\":{}}}", "{\"\":1}", "-", "1e", "1e+", "--1", "1.5e3" } do
	table.insert(Parts, string.format("%q", Text) .. "=" .. Decode(Text))
end
print("decode-more " .. table.concat(Parts, " "))
print("decode-args " .. Decode(nil) .. " | " .. Decode(true) .. " | " .. Decode(12.5) .. " | " .. Decode({}))
