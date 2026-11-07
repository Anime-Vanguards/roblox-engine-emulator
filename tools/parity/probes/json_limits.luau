-- Where JSONEncode's and JSONDecode's nesting limits fall (found by bisection), whether they move with the caller's own
-- stack depth, and what JSONDecode does with an argument that is not a string.
local Http = game:GetService("HttpService")
local function Nest(Depth, Key)
	local Root = {}
	local At = Root
	for _ = 2, Depth do
		At[Key or 1] = {}
		At = At[Key or 1]
	end
	return Root
end
-- the largest depth in [Low, High) that works, given that Low works and High does not
local function Largest(Works, Low, High)
	while High - Low > 1 do
		local Middle = (Low + High) // 2
		if Works(Middle) then
			Low = Middle
		else
			High = Middle
		end
	end
	return Low
end
local function Encodes(Depth)
	return (pcall(Http.JSONEncode, Http, Nest(Depth)))
end
print("encode-depth " .. Largest(Encodes, 300, 500))
print("encode-dict-depth " .. Largest(function(Depth)
	return (pcall(Http.JSONEncode, Http, Nest(Depth, "a")))
end, 300, 500))
local function Deeper(Level)
	if Level > 0 then
		local Found = Deeper(Level - 1)
		return Found
	end
	return Largest(Encodes, 100, 500)
end
print("encode-depth-from-deeper " .. Deeper(60))
local function Document(Depth)
	return string.rep("[", Depth) .. string.rep("]", Depth)
end
print("decode-convert-depth " .. Largest(function(Depth)
	return (pcall(Http.JSONDecode, Http, Document(Depth)))
end, 257, 500))
print("decode-parse-depth " .. Largest(function(Depth)
	local Ok, Message = pcall(Http.JSONDecode, Http, Document(Depth))
	return Ok or not string.find(tostring(Message), "Can't parse", 1, true)
end, 1000, 5000))
print("decode-dict-convert-depth " .. Largest(function(Depth)
	return (pcall(Http.JSONDecode, Http, string.rep("{\"a\":", Depth) .. "1" .. string.rep("}", Depth)))
end, 100, 500))

-- JSONDecode of what is not a string
local Input = { 1, 2 }
local Ok, Result = pcall(Http.JSONDecode, Http, Input)
print("decode-table " .. tostring(Ok) .. " " .. type(Result) .. " same=" .. tostring(rawequal(Result, Input)) .. " #" .. (if type(Result) == "table" then #Result else -1))
for _, Pair in { { "dict", { a = 1 } }, { "instance", workspace }, { "vector3", Vector3.new(1, 2, 3) }, { "function", print }, { "false", false }, { "string-number", "5" } } do
	local Decoded, Value = pcall(Http.JSONDecode, Http, Pair[2])
	print("decode-" .. Pair[1] .. " " .. tostring(Decoded) .. " " .. typeof(Value) .. " " .. (if type(Value) == "table" then "same=" .. tostring(rawequal(Value, Pair[2])) else P.text(P.err(tostring(Value)))))
end
