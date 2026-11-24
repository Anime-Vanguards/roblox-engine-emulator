-- EncodingService:ComputeBufferHash and ComputeStringHash: what each HashAlgorithm answers (the answer's type, its length
-- and its bytes as hex) for inputs around every block size the algorithms use, large inputs and the empty one, and the
-- errors and warnings of wrong arguments. The inputs are made at run time: byte i of a patterned input is i % 251.
local Encoding = game:GetService("EncodingService")
local LogService = game:GetService("LogService")

local Seen = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageWarning and string.sub(Message, 1, 10) == "argument #" then
		table.insert(Seen, "[" .. (string.gsub(Message, "\n.*", "")) .. "]")
	end
end)

local function hex(Text)
	return (string.gsub(Text, ".", function(Byte)
		return string.format("%02x", string.byte(Byte))
	end))
end
local function show(Value)
	local Kind = typeof(Value)
	if Kind == "buffer" then
		return "buffer[" .. buffer.len(Value) .. "] " .. hex(buffer.tostring(Value))
	elseif Kind == "string" then
		return "string[" .. #Value .. "] " .. hex(Value)
	elseif Kind == "number" then
		return "number " .. P.num(Value)
	end
	return Kind .. " " .. tostring(Value)
end
local function pattern(Length)
	local Data = buffer.create(Length)
	for Index = 0, Length - 1 do
		buffer.writeu8(Data, Index, Index % 251)
	end
	return Data
end

local Algorithms = Enum.HashAlgorithm:GetEnumItems()
table.sort(Algorithms, function(A, B)
	return A.Value < B.Value
end)
local Names = {}
for _, Algorithm in Algorithms do
	table.insert(Names, Algorithm.Name .. "=" .. Algorithm.Value)
end
print("algorithms", table.concat(Names, " "))

-- ComputeBufferHash: every algorithm, every length
local Lengths = { 0, 1, 2, 3, 31, 32, 33, 55, 56, 57, 63, 64, 65, 111, 112, 113, 127, 128, 129, 255, 256, 257, 1023, 1024, 1025,
	2048, 2049, 3072, 3073, 4096, 4097, 5120, 5121, 8192, 8193, 16384, 31744, 102400 }
local Inputs = {}
for _, Length in Lengths do
	table.insert(Inputs, { "pattern" .. Length, pattern(Length) })
end
table.insert(Inputs, { "abc", buffer.fromstring("abc") })
table.insert(Inputs, { "fox", buffer.fromstring("The quick brown fox jumps over the lazy dog") })
table.insert(Inputs, { "zeros", buffer.fromstring("a\0b\0") })
local Million = buffer.create(1000000)
buffer.fill(Million, 0, string.byte("a"))
table.insert(Inputs, { "million-a", Million })
table.insert(Inputs, { "pattern1048577", pattern(1048577) })
local Large = buffer.create(16 * 1048576 + 1)
buffer.fill(Large, 0, 0x5a)
table.insert(Inputs, { "fill16777217", Large })
for _, Algorithm in Algorithms do
	for _, Input in Inputs do
		print("buffer", Algorithm.Name, Input[1], P.try(function()
			return show(Encoding:ComputeBufferHash(Input[2], Algorithm))
		end))
	end
end

-- ComputeStringHash: what it answers, for the texts the buffers above hold
local Texts = {
	{ "empty", "" },
	{ "abc", "abc" },
	{ "zeros", "a\0b\0" },
	{ "pattern1025", buffer.tostring(pattern(1025)) },
	{ "million-a", string.rep("a", 1000000) },
}
for _, Algorithm in Algorithms do
	for _, Text in Texts do
		print("string", Algorithm.Name, Text[1], P.try(function()
			return show(Encoding:ComputeStringHash(Text[2], Algorithm))
		end))
	end
end

-- each call answers a buffer of its own
local Abc = buffer.fromstring("abc")
local First = Encoding:ComputeBufferHash(Abc, Enum.HashAlgorithm.Sha256)
buffer.writeu8(First, 0, 0)
local Second = Encoding:ComputeBufferHash(Abc, Enum.HashAlgorithm.Sha256)
print("fresh", rawequal(First, Second), buffer.readu8(Second, 0), buffer.tostring(Abc))

-- wrong arguments: the call's outcome, then what the engine warned
local function call(Label, Function)
	local Result = P.try(Function)
	task.wait()
	task.wait()
	print(Label, Result, table.concat(Seen, " "))
	table.clear(Seen)
end
local Sha256 = Enum.HashAlgorithm.Sha256
local Md5 = Enum.HashAlgorithm.Md5
call("buffer-none", function()
	return show(Encoding:ComputeBufferHash())
end)
call("buffer-extra", function()
	return show(Encoding:ComputeBufferHash(Abc, Md5, 1, "x"))
end)
call("buffer-dot", function()
	return show(Encoding.ComputeBufferHash(Abc, Sha256))
end)
call("string-none", function()
	return show(Encoding:ComputeStringHash())
end)
call("string-both-wrong", function()
	return show(Encoding:ComputeStringHash(true, "Nope"))
end)
call("string-nil-both", function()
	return show(Encoding:ComputeStringHash(nil, nil))
end)
call("string-dot", function()
	return show(Encoding.ComputeStringHash("abc", Sha256))
end)

-- the input: what each method takes (a string given a value that is no string is given its text, after a warning)
local Values = {
	{ "nil", nil },
	{ "string", "abc" },
	{ "buffer", Abc },
	{ "int", 5 },
	{ "tenth", 0.1 },
	{ "nan", 0 / 0 },
	{ "2^63", 2 ^ 63 },
	{ "true", true },
	{ "table", {} },
	{ "instance", workspace },
	{ "vector", Vector3.new(1, 2, 3) },
	{ "cframe", CFrame.new() },
	{ "enumitem", Md5 },
	{ "enum", Enum.HashAlgorithm },
	{ "function", print },
}
for _, Case in Values do
	call("buffer-input " .. Case[1], function()
		return show(Encoding:ComputeBufferHash(Case[2], Md5))
	end)
	call("string-input " .. Case[1], function()
		return show(Encoding:ComputeStringHash(Case[2], Md5))
	end)
end

-- the algorithm: an item of HashAlgorithm, an item's exact name, or a number cut to a whole one that is an item's value
local Algorithms = {
	{ "nil", nil },
	{ "zstd", Enum.CompressionAlgorithm.Zstd },
	{ "enum", Enum.HashAlgorithm },
	{ "enums", Enum },
	{ "value-0", 0 },
	{ "value-4", 4 },
	{ "value-5", 5 },
	{ "value-2.5", 2.5 },
	{ "value-4.9", 4.9 },
	{ "value-neg-half", -0.5 },
	{ "value--1", -1 },
	{ "value-2^32+4", 2 ^ 32 + 4 },
	{ "value-nan", 0 / 0 },
	{ "value-inf", math.huge },
	{ "name", "Sha1" },
	{ "name-lower", "sha1" },
	{ "name-number", "4" },
	{ "bad-name", "Nope" },
	{ "true", true },
	{ "buffer", Abc },
	{ "vector", Vector3.zero },
	{ "cframe", CFrame.new() },
	{ "function", print },
	{ "thread", coroutine.create(print) },
	{ "newproxy", newproxy() },
	{ "instance", workspace },
	{ "dictionary", { a = 1 } },
	{ "array", { 1 } },
}
for _, Case in Algorithms do
	call("buffer-algorithm " .. Case[1], function()
		return show(Encoding:ComputeBufferHash(Abc, Case[2]))
	end)
	call("string-algorithm " .. Case[1], function()
		return show(Encoding:ComputeStringHash("abc", Case[2]))
	end)
end
Listener:Disconnect()
