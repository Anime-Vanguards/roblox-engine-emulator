-- How HttpService:JSONEncode writes numbers and JSONDecode reads them: 600 doubles made from random bits (every
-- magnitude, integers, short decimals), each encoded; then the decoded value of long digit strings, read back exactly.
local Http = game:GetService("HttpService")
local Rng = P.rng(4242)
local function Double(Low, High)
	return (string.unpack("<d", string.pack("<I4I4", Low, High)))
end
local Values = {}
for _ = 1, 400 do
	-- any finite double: random mantissa, random exponent
	local High = Rng.u32()
	local Exponent = bit32.extract(High, 20, 11)
	if Exponent == 2047 then
		High = bit32.replace(High, 1023, 20, 11)
	end
	table.insert(Values, Double(Rng.u32(), High))
end
for _ = 1, 100 do
	-- integers of every size up to 2^60, and halves of them
	local Bits = Rng.int(1, 60)
	local Value = (Rng.u32() % 2 ^ math.min(Bits, 32)) * 2 ^ math.max(Bits - 32, 0) + Rng.u32() % 2 ^ math.max(Bits - 32, 0)
	table.insert(Values, if Rng.int(0, 3) == 0 then Value / 2 else Value)
end
for _ = 1, 100 do
	-- decimals as a game writes them: up to six places, and a value made by arithmetic
	local Places = Rng.int(1, 6)
	local Value = Rng.int(-100000, 100000) / 10 ^ Places
	table.insert(Values, if Rng.int(0, 1) == 0 then Value else Value * 3 / 7)
end
for Line = 0, #Values / 25 - 1 do
	local Out = {}
	for Index = Line * 25 + 1, Line * 25 + 25 do
		local Ok, Text = pcall(Http.JSONEncode, Http, Values[Index])
		table.insert(Out, if Ok then Text else "E")
	end
	print("encode " .. Line + 1 .. " " .. table.concat(Out, " "))
end
-- decoding: long and awkward digit strings, read back as doubles
local Texts = { "0.30000000000000004", "0.30000000000000007", "195.66666666666666", "1.7976931348623157e308", "2.2250738585072014e-308", "4.9e-324",
	"2.4703282292062327e-324", "9007199254740993", "9007199254740992.5", "123456789012345678901234567890", "0.1000000000000000055511151231257827",
	"1e23", "8.98846567431158e307", "1.00000000000000011102230246251565404236316680908203125", "1.000000000000000111022302462515654042363166809082031251",
	"0.000001", "1e-7", "-0.0", "-0", "0e10", "1E-2", "12345678901234567890e-30" }
local Read = {}
for _, Text in Texts do
	local Ok, Value = pcall(Http.JSONDecode, Http, Text)
	table.insert(Read, Text .. "=" .. (if Ok then P.num(Value) else "E(" .. P.err(Value) .. ")"))
end
print("decode " .. table.concat(Read, " "))
-- and each generated value through a round trip: encoded, decoded, the same double?
local Same, Differ = 0, {}
for Index, Value in Values do
	local Ok, Back = pcall(function()
		return Http:JSONDecode(Http:JSONEncode(Value))
	end)
	if Ok and Back == Value then
		Same += 1
	else
		table.insert(Differ, Index)
	end
end
print("roundtrip same " .. Same .. " of " .. #Values .. " differ " .. table.concat(Differ, ","))
