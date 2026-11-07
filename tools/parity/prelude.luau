-- Prepended to every parity probe, in ree and in the real engine alike. A probe prints canonical lines through these
-- helpers, so the two outputs compare line by line: numbers in a form both engines print the same way, error messages
-- with the probe's own chunk name and line offset taken out, and cases drawn from a generator of our own (Random and
-- math.random are under test themselves).
local __ParityBase = 0 -- set to the line above the probe's first line (run.mjs body)
local __ParityChunk = debug.info(1, "s")
local P = { base = __ParityBase }

-- Printed text as valid UTF-8 without control bytes: a byte that does not start a valid sequence, and every control byte
-- but tab, becomes \xNN (a real engine's test runner carries output as JSON, which takes UTF-8 only).
function P.text(Text)
	local Out = {}
	local Position = 1
	while Position <= #Text do
		local Length, Bad = utf8.len(Text, Position)
		if Length then
			table.insert(Out, string.sub(Text, Position))
			break
		end
		table.insert(Out, string.sub(Text, Position, Bad - 1))
		table.insert(Out, string.format("\\x%02X", string.byte(Text, Bad)))
		Position = Bad + 1
	end
	local Joined = string.gsub(table.concat(Out), "\237[\160-\191][\128-\191]", function(Surrogate)
		-- an encoded UTF-16 surrogate: Luau's utf8 accepts it, strict UTF-8 readers do not
		return string.format("\\x%02X\\x%02X\\x%02X", string.byte(Surrogate, 1, 3))
	end)
	return (string.gsub(Joined, "[%z\1-\8\10-\31\127]", function(Byte)
		return string.format("\\x%02X", string.byte(Byte))
	end))
end
local __ParityPrint = print
local function print(...)
	local Parts = table.pack(...)
	for Index = 1, Parts.n do
		Parts[Index] = P.text(tostring(Parts[Index]))
	end
	__ParityPrint(table.concat(Parts, " ", 1, Parts.n))
end

-- A number as text: %.17g for finite values (round-trips every double), with NaN, infinities and -0 spelled out,
-- since C libraries print those differently.
function P.num(Value)
	if type(Value) ~= "number" then
		return tostring(Value)
	elseif Value ~= Value then
		return "nan"
	elseif Value == math.huge then
		return "inf"
	elseif Value == -math.huge then
		return "-inf"
	elseif Value == 0 then
		return if 1 / Value < 0 then "-0" else "0"
	end
	return string.format("%.17g", Value)
end

-- A 32-bit float as text: 9 significant digits round-trip it (CFrame, Vector3 and Color3 hold floats).
function P.f32(Value)
	if type(Value) ~= "number" or Value ~= Value or Value == math.huge or Value == -math.huge or Value == 0 then
		return P.num(Value)
	end
	return string.format("%.9g", Value)
end

function P.nums(...)
	local Parts = table.pack(...)
	for Index = 1, Parts.n do
		Parts[Index] = P.num(Parts[Index])
	end
	return table.concat(Parts, " ", 1, Parts.n)
end

function P.f32s(...)
	local Parts = table.pack(...)
	for Index = 1, Parts.n do
		Parts[Index] = P.f32(Parts[Index])
	end
	return table.concat(Parts, " ", 1, Parts.n)
end

-- An error message with the probe's own position made comparable: "<this chunk>:<line>:" becomes "@<line in the
-- probe>:", whatever the chunk is called and whatever runs above the probe.
function P.err(Message)
	local Text = tostring(Message)
	local Escaped = string.gsub(__ParityChunk, "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
	Text = string.gsub(Text, Escaped .. ":(%d+):", function(Line)
		return "@" .. tostring(tonumber(Line) - P.base) .. ":"
	end)
	return Text
end

-- A traceback as one line: its message, then the probe's own frames as "@<line>[ function <name>]" (frames of whatever
-- runs the probe differ between engines and are left out).
function P.trace(Text)
	local Out = {}
	local Prefix = __ParityChunk .. ":"
	for Index, Line in string.split(tostring(Text), "\n") do
		local Number, Rest
		if string.sub(Line, 1, #Prefix) == Prefix then
			Number, Rest = string.match(string.sub(Line, #Prefix + 1), "^(%d+)(.*)$")
		end
		if Number and string.sub(Rest, 1, 1) ~= ":" then
			-- a frame of the probe
			table.insert(Out, "@" .. tostring(tonumber(Number) - P.base) .. Rest)
		elseif Index == 1 then
			-- the message
			table.insert(Out, P.err(Line))
		end
	end
	return table.concat(Out, " / ")
end

-- The name of the chunk the probe runs in, for normalizing engine messages that name it.
function P.chunk()
	return __ParityChunk
end

-- pcall's outcome as one line: "ok <values>" or "error <message>".
function P.try(Function, ...)
	local Results = table.pack(pcall(Function, ...))
	if Results[1] then
		local Parts = {}
		for Index = 2, Results.n do
			local Value = Results[Index]
			local Kind = type(Value)
			if Kind == "number" then
				table.insert(Parts, P.num(Value))
			elseif Kind == "string" or Kind == "boolean" or Kind == "nil" then
				table.insert(Parts, tostring(Value))
			else
				-- addresses differ run to run
				table.insert(Parts, (string.gsub(tostring(Value), "0x%x+", "ADDR")))
			end
		end
		return "ok " .. table.concat(Parts, " ")
	end
	return "error " .. P.err(Results[2])
end

-- The probe's line of the caller (the same in both engines).
function P.line(Level)
	return debug.info((Level or 1) + 1, "l") - P.base
end

-- xorshift32 cases: the same sequence in every engine, from bit32 alone.
function P.rng(Seed)
	local State = bit32.band(Seed, 0xFFFFFFFF)
	if State == 0 then
		State = 0x9E3779B9
	end
	local Generator = {}
	function Generator.u32()
		State = bit32.bxor(State, bit32.lshift(State, 13))
		State = bit32.bxor(State, bit32.rshift(State, 17))
		State = bit32.bxor(State, bit32.lshift(State, 5))
		return State
	end
	-- an integer in [Low, High]
	function Generator.int(Low, High)
		return Low + Generator.u32() % (High - Low + 1)
	end
	-- a float in [Low, High) on a 1/65536 grid (exact in floats and doubles alike)
	function Generator.real(Low, High)
		return Low + (High - Low) * (Generator.u32() % 65536) / 65536
	end
	function Generator.pick(List)
		return List[Generator.u32() % #List + 1]
	end
	return Generator
end

-- Frames: Heartbeats counted from now.
function P.frames()
	local RunService = game:GetService("RunService")
	local Count = 0
	local Connection = RunService.Heartbeat:Connect(function()
		Count += 1
	end)
	return function()
		return Count
	end, Connection
end

-- Start at an engine resumption point: a task.wait resumes from the scheduler in both engines (a probe's first line
-- may run from anywhere: a harness before the first frame, a test job mid-frame).
function P.sync()
	task.wait()
	task.wait()
end
