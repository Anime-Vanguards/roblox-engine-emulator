-- Subnormal numbers (below the smallest normal double, 2^-1022, or float32, 2^-126): which steps make one zero and which keep
-- it. Each line prints a case's bits (a double high byte first, a float32 as the bytes written), first from values made at run
-- time, then from literals the compiler folds where it can. The run-time values are a cloned table's fields: the compiler folds
-- the constant fields of a local table that never escapes, so a plain `local K = { ... }` would fold too. A float mode that
-- flushes results to zero (x86 FTZ) makes a subnormal result zero; one that also reads subnormal inputs as zero (DAZ) would
-- break the reads, comparisons and scaling lines too.
local K = table.clone({
	Sub = 5e-324, -- the smallest subnormal double
	Sub2 = 1e-310,
	MinNormal = 2.2250738585072014e-308,
	One = 1,
	Two = 2,
	Big = 2 ^ 60,
	SubText = "5e-324",
	F32Sub = 1e-45, -- rounds to the smallest subnormal float32
	F32Sub2 = 1e-39,
	F32MinNormal = 1.1754943508222875e-38,
	Tiny = 1e-20, -- its square, 1e-40, is a subnormal float32
	F32Scale = 2 ^ 100,
})

local function Hex(Text)
	return (string.gsub(Text, ".", function(Character)
		return string.format("%02X", string.byte(Character))
	end))
end

-- a double's bits, high byte first (string.pack copies them: no arithmetic)
local function D(Value)
	if type(Value) ~= "number" then
		return type(Value)
	end
	return Hex(string.pack(">d", Value))
end

-- the float32 buffer.writef32 stores, as its bytes
local function W32(Value)
	local Target = buffer.create(4)
	buffer.writef32(Target, 0, Value)
	return Hex(buffer.readstring(Target, 0, 4))
end

-- a vector's components, widened to doubles
local function V(Value)
	return D(Value.X) .. " " .. D(Value.Y) .. " " .. D(Value.Z)
end

-- literals, text and the C library: no arithmetic on a subnormal result
print("literal " .. D(K.Sub) .. " " .. D(K.Sub2) .. " " .. D(K.MinNormal) .. " | " .. D(5e-324) .. " " .. D(1e-310) .. " " .. D(-5e-324))
print("tonumber " .. D(tonumber(K.SubText)) .. " " .. D(tonumber("0x1p-1074")) .. " | " .. D(tonumber("5e-324")))
print("tostring " .. tostring(K.Sub) .. " " .. tostring(K.Sub2) .. " " .. tostring(-K.Sub) .. " | " .. tostring(5e-324))
print("ldexp " .. D(math.ldexp(K.One, -1074)) .. " " .. D(math.ldexp(K.One, -1030)) .. " | " .. D(math.ldexp(1, -1074)) .. " " .. D(math.ldexp(1, -1030)))
print("negate " .. D(-K.Sub) .. " " .. D(-K.Sub2) .. " | " .. D(-5e-324))
print("format " .. string.format("%.17g", K.Sub) .. " " .. string.format("%.17g", K.Sub2) .. " " .. string.format("%g", K.Sub2) .. " " .. string.format("%.3e", K.Sub2) .. " | " .. string.format("%.17g", 5e-324))
print("format-normal " .. string.format("%.17g", K.MinNormal) .. " " .. string.format("%.17g", K.MinNormal * K.Two))

-- double arithmetic with a subnormal result
print("mul " .. D(K.Sub * K.One) .. " " .. D(K.Sub2 * K.One) .. " " .. D(K.MinNormal * 0.5) .. " | " .. D(5e-324 * 1) .. " " .. D(1e-310 * 1))
print("add " .. D(K.Sub + K.Sub) .. " " .. D(K.Sub2 + K.Sub2) .. " " .. D(K.Sub + 0) .. " | " .. D(5e-324 + 5e-324) .. " " .. D(5e-324 + 0))
print("sub " .. D(K.MinNormal * 1.5 - K.MinNormal) .. " " .. D(K.Sub2 - K.Sub) .. " | " .. D(2.2250738585072014e-308 * 1.5 - 2.2250738585072014e-308))
print("div " .. D(K.MinNormal / K.Two) .. " " .. D(K.MinNormal / 1e10) .. " | " .. D(2.2250738585072014e-308 / 2) .. " " .. D(2.2250738585072014e-308 / 1e10))
print("pow " .. D(K.Two ^ -1074) .. " " .. D(K.Two ^ -1030) .. " | " .. D(2 ^ -1074) .. " " .. D(2 ^ -1030))
print("mod " .. D(K.Sub2 % K.One) .. " | " .. D(1e-310 % 1))
print("math " .. D(math.abs(K.Sub)) .. " " .. D(math.abs(-K.Sub2)) .. " " .. D(math.min(K.Sub, K.One)) .. " " .. D(math.max(-K.One, K.Sub)) .. " " .. D(math.floor(K.Sub)) .. " " .. D(math.clamp(K.Sub, 0, 1)) .. " | " .. D(math.abs(5e-324)) .. " " .. D(math.min(5e-324, 1)))

-- a subnormal input with a normal result (read as zero only by a mode that also flushes inputs)
print("scale-up " .. D(K.Sub * K.Big) .. " " .. D(K.Sub2 * K.Big) .. " " .. D(K.Sub / 2 ^ -60) .. " | " .. D(5e-324 * 2 ^ 60))
print("sqrt " .. D(math.sqrt(K.Sub)) .. " " .. D(math.sqrt(K.Sub2)) .. " | " .. D(math.sqrt(5e-324)))
print("compare " .. tostring(K.Sub > 0) .. " " .. tostring(K.Sub == 0) .. " " .. tostring(K.Sub ~= 0) .. " " .. tostring(K.Sub < K.MinNormal) .. " " .. tostring(-K.Sub < 0) .. " " .. tostring(K.Sub == K.Sub2) .. " | " .. tostring(5e-324 > 0) .. " " .. tostring(5e-324 == 0))
print("classify " .. tostring(K.Sub ~= K.Sub) .. " " .. tostring(K.Sub == math.huge) .. " " .. tostring(math.sign(K.Sub)) .. " " .. tostring(math.sign(-K.Sub)) .. " " .. tostring(K.Sub > -K.Sub))

-- float32 conversions: a double whose float32 is subnormal
print("writef32 " .. W32(K.F32Sub) .. " " .. W32(K.F32Sub2) .. " " .. W32(-K.F32Sub2) .. " " .. W32(K.F32MinNormal) .. " | " .. W32(1e-45) .. " " .. W32(1e-39))
print("pack-f " .. Hex(string.pack("<f", K.F32Sub)) .. " " .. Hex(string.pack("<f", K.F32Sub2)) .. " | " .. Hex(string.pack("<f", 1e-45)) .. " " .. Hex(string.pack("<f", 1e-39)))
print("vector-create " .. V(vector.create(K.F32Sub, K.F32Sub2, -K.F32Sub2)) .. " | " .. V(vector.create(1e-45, 1e-39, -1e-39)))
print("vector3-new " .. V(Vector3.new(K.F32Sub, K.F32Sub2, -K.F32Sub2)) .. " | " .. V(Vector3.new(1e-45, 1e-39, -1e-39)))
print("vector3-normal " .. V(Vector3.new(K.F32MinNormal, 0, 0)) .. " | " .. V(Vector3.new(1.1754943508222875e-38, 0, 0)))

-- float32 arithmetic with a subnormal result
local A = vector.create(K.Tiny, K.Tiny, 0)
print("vector-mul " .. V(A * A) .. " " .. V(A * K.Tiny) .. " " .. V(A / 1e20) .. " | " .. V(vector.create(1e-20, 1e-20, 0) * vector.create(1e-20, 1e-20, 0)) .. " " .. V(vector.create(1e-20, 0, 0) * 1e-20))
print("vector-add " .. V(vector.create(K.F32MinNormal * 1.5, 0, 0) - vector.create(K.F32MinNormal, 0, 0)) .. " | " .. V(vector.create(1.1754943508222875e-38 * 1.5, 0, 0) - vector.create(1.1754943508222875e-38, 0, 0)))
print("magnitude " .. D(Vector3.new(K.Tiny, 0, 0).Magnitude) .. " " .. D(vector.magnitude(A)) .. " " .. D(Vector3.new(K.Tiny, 0, 0):Dot(Vector3.new(K.Tiny, 0, 0))) .. " " .. V(Vector3.new(K.Tiny, K.Tiny, 0).Unit) .. " | " .. D(Vector3.new(1e-20, 0, 0).Magnitude) .. " " .. D(vector.magnitude(vector.create(1e-20, 1e-20, 0))))
print("vector-cross " .. V(Vector3.new(K.Tiny, 0, 0):Cross(Vector3.new(0, K.Tiny, 0))) .. " " .. D(vector.dot(A, A)))

-- reads of a subnormal float32, and float32 arithmetic with a subnormal input and a normal result
local Stored = buffer.create(8)
buffer.writeu32(Stored, 0, 0x00000001)
buffer.writeu32(Stored, 4, 0x80400000)
local Read = vector.create(buffer.readf32(Stored, 0), buffer.readf32(Stored, 4), 0)
print("readf32 " .. D(buffer.readf32(Stored, 0)) .. " " .. D(buffer.readf32(Stored, 4)) .. " " .. D((string.unpack("<f", "\1\0\0\0"))) .. " " .. D((string.unpack("<f", buffer.readstring(Stored, 4, 4)))))
print("read-vector " .. V(Read) .. " " .. V(Read * K.F32Scale) .. " " .. tostring(Read.X > 0) .. " " .. tostring(Read == vector.zero))
print("read-scale " .. D(buffer.readf32(Stored, 0) * K.F32Scale) .. " " .. W32(buffer.readf32(Stored, 0)) .. " " .. Hex(string.pack("<f", buffer.readf32(Stored, 4))))

-- the engine's float32 datatypes and properties
local Part = Instance.new("Part")
Part.Transparency = K.F32Sub2
local Number = Instance.new("NumberValue")
Number.Value = K.Sub
print("datatypes " .. D(CFrame.new(K.F32Sub2, 0, 0).X) .. " " .. D(Color3.new(K.F32Sub2, 0, 0).R) .. " " .. D(Vector2.new(K.F32Sub2, 0).X) .. " " .. D(UDim.new(K.F32Sub2, 0).Scale) .. " | " .. D(CFrame.new(1e-39, 0, 0).X))
print("properties " .. D(Part.Transparency) .. " " .. D(Number.Value) .. " " .. D(Instance.new("Vector3Value").Value.X))
print("cframe-mul " .. D((CFrame.new(K.Tiny, 0, 0) * CFrame.new(0, 0, 0)).X) .. " " .. D(CFrame.new(0, 0, 0):PointToWorldSpace(Vector3.new(K.F32Sub2, 0, 0)).X) .. " " .. D((CFrame.Angles(0, K.Tiny, 0) * Vector3.new(K.Tiny, 0, 0)).Z))

-- JSON
local Http = game:GetService("HttpService")
print("json " .. D(Http:JSONDecode("[5e-324]")[1]) .. " " .. D(Http:JSONDecode("[1e-310]")[1]) .. " " .. Http:JSONEncode({ K.Sub2 }) .. " " .. Http:JSONEncode({ K.MinNormal }))

-- the same on every path a script's code runs on: in a coroutine, under pcall, in a spawned thread; once resumed, in a deferred
-- handler and thread, a Heartbeat handler and a thread Heartbeat resumes; a tween's Changed handlers; and on a client, its render
-- step (RenderStepped, PreRender, a binding, what they defer, a thread RenderStepped resumes, one they make wait)
local function Sample()
	return D(K.Sub * K.One) .. " " .. W32(K.F32Sub) .. " " .. D(vector.create(K.F32Sub2, 0, 0).X) .. " " .. D(buffer.readf32(Stored, 0)) .. " " .. tostring(K.Sub > 0)
end
local RunService = game:GetService("RunService")
local Lines = {}
print("path main " .. Sample())
print("path coroutine " .. coroutine.wrap(Sample)())
print("path pcall " .. select(2, pcall(Sample)))
task.spawn(function()
	Lines.spawned = Sample()
end)
print("path spawned " .. tostring(Lines.spawned))
task.wait()
print("path resumed " .. Sample())
local Event = Instance.new("BindableEvent")
Event.Event:Connect(function()
	Lines.handler = Sample()
end)
Event:Fire()
task.defer(function()
	Lines.deferred = Sample()
end)
local Beat = RunService.Heartbeat:Connect(function()
	Lines.heartbeat = Lines.heartbeat or Sample()
end)
task.spawn(function()
	RunService.Heartbeat:Wait()
	Lines.heartbeatWait = Sample()
end)
local Tweened = Instance.new("NumberValue")
Tweened.Changed:Connect(function()
	Lines.tween = Lines.tween or Sample()
end)
game:GetService("TweenService"):Create(Tweened, TweenInfo.new(0.1, Enum.EasingStyle.Linear), { Value = 1 }):Play()
local Client = RunService:IsClient()
local Render
if Client then
	Render = RunService.RenderStepped:Connect(function()
		if Lines.render then
			return
		end
		Lines.render = Sample()
		task.defer(function()
			Lines.renderDeferred = Sample()
		end)
		task.spawn(function()
			task.wait()
			Lines.renderWait = Sample()
		end)
	end)
	RunService:BindToRenderStep("FloatFlushProbe", Enum.RenderPriority.Camera.Value, function()
		Lines.binding = Lines.binding or Sample()
	end)
	task.spawn(function()
		RunService.PreRender:Wait()
		Lines.preRenderWait = Sample()
	end)
end
for _ = 1, 10 do
	task.wait()
end
Beat:Disconnect()
if Client then
	Render:Disconnect()
	RunService:UnbindFromRenderStep("FloatFlushProbe")
end
for _, Name in { "handler", "deferred", "heartbeat", "heartbeatWait", "tween", "render", "renderDeferred", "renderWait", "binding", "preRenderWait" } do
	if Client or not string.find(Name, "ender") and Name ~= "binding" then
		print("path " .. Name .. " " .. tostring(Lines[Name]))
	end
end

-- a chunk compiled at run time (a server with LoadStringEnabled; a client has no loadstring)
if game:GetService("RunService"):IsServer() then
	print("loadstring " .. P.try(function()
		local Chunk = loadstring("return 2 ^ -1074, 5e-324 * 1, vector.create(1e-45, 0, 0).X, Vector3.new(1e-39, 0, 0).X")
		local Pow, Mul, X, Y = Chunk()
		return D(Pow) .. " " .. D(Mul) .. " " .. D(X) .. " " .. D(Y)
	end))
end
