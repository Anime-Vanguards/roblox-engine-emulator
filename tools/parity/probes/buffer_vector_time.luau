-- The buffer and vector libraries, os and DateTime at fixed times.
-- buffer
local Buffer = buffer.create(16)
buffer.writei8(Buffer, 0, -1)
buffer.writeu16(Buffer, 1, 65535)
buffer.writei32(Buffer, 4, -123456)
buffer.writef32(Buffer, 8, 1 / 3)
print("buffer-reads " .. P.nums(buffer.readu8(Buffer, 0), buffer.readi8(Buffer, 0), buffer.readu16(Buffer, 1), buffer.readi16(Buffer, 1), buffer.readi32(Buffer, 4), buffer.readu32(Buffer, 4), buffer.readf32(Buffer, 8)))
buffer.writef64(Buffer, 8, 0 / 0)
print("buffer-nan " .. P.num(buffer.readf64(Buffer, 8)) .. " bytes " .. string.format("%q", string.sub(buffer.tostring(Buffer), 9, 16)))
buffer.writef32(Buffer, 0, 0 / 0)
print("buffer-f32-nan-bytes " .. table.concat({ buffer.readu8(Buffer, 0), buffer.readu8(Buffer, 1), buffer.readu8(Buffer, 2), buffer.readu8(Buffer, 3) }, ","))
buffer.writef32(Buffer, 0, 1e40)
print("buffer-f32-overflow " .. P.num(buffer.readf32(Buffer, 0)))
buffer.writef32(Buffer, 0, 1e-46)
print("buffer-f32-underflow " .. P.num(buffer.readf32(Buffer, 0)))
buffer.writeu8(Buffer, 0, 300)
buffer.writei8(Buffer, 1, 200)
buffer.writeu32(Buffer, 4, -1)
buffer.writeu32(Buffer, 8, 2 ^ 33 + 7)
buffer.writeu8(Buffer, 12, 2.9)
buffer.writeu8(Buffer, 13, -2.9)
print("buffer-wrap " .. P.nums(buffer.readu8(Buffer, 0), buffer.readi8(Buffer, 1), buffer.readu32(Buffer, 4), buffer.readu32(Buffer, 8), buffer.readu8(Buffer, 12), buffer.readu8(Buffer, 13)))
print("buffer-oob " .. P.try(buffer.readu8, Buffer, 16) .. " | " .. P.try(buffer.readu32, Buffer, 13) .. " | " .. P.try(buffer.writeu8, Buffer, -1, 0) .. " | " .. P.try(buffer.readu8, Buffer, 1.5))
print("buffer-string " .. P.try(function()
	local Text = buffer.fromstring("hello")
	buffer.writestring(Text, 1, "EL")
	return buffer.tostring(Text) .. " " .. buffer.len(Text) .. " " .. buffer.readstring(Text, 1, 3)
end))
print("buffer-writestring-count " .. P.try(function()
	local Text = buffer.create(4)
	buffer.writestring(Text, 0, "abcdef", 2)
	return buffer.tostring(Text)
end) .. " | " .. P.try(buffer.writestring, buffer.create(2), 0, "abc"))
print("buffer-copy-fill " .. P.try(function()
	local A = buffer.fromstring("abcdef")
	buffer.copy(A, 2, A, 0, 3)
	local B = buffer.create(4)
	buffer.fill(B, 1, 65, 2)
	return buffer.tostring(A) .. " " .. string.format("%q", buffer.tostring(B))
end))
print("buffer-create-errors " .. P.try(buffer.create, -1) .. " | " .. P.try(buffer.create, 2 ^ 31) .. " | " .. P.try(function()
	return buffer.len(buffer.create(1.5))
end))
print("buffer-bits " .. P.try(function()
	local B = buffer.create(8)
	buffer.writebits(B, 3, 10, 0x2AB)
	return buffer.readbits(B, 3, 10) .. " " .. buffer.readu16(B, 0)
end))
print("buffer-type " .. typeof(Buffer) .. " " .. type(Buffer) .. " " .. string.gsub(tostring(Buffer), "0x%x+", "ADDR"))
print("buffer-equality " .. tostring(buffer.fromstring("a") == buffer.fromstring("a")))

-- vector
local V = vector.create(1, 2, 3)
print("vector " .. typeof(V) .. " " .. type(V) .. " " .. tostring(V) .. " " .. P.f32s(V.x, V.y, V.z, V.X))
print("vector-ops " .. tostring(V + V) .. " | " .. tostring(V * 2) .. " | " .. tostring(V / 0) .. " | " .. tostring(-V) .. " | " .. tostring(V * V) .. " | " .. tostring(V // 2))
print("vector-lib " .. P.f32s(vector.magnitude(V), vector.dot(V, V)) .. " " .. tostring(vector.normalize(V)) .. " " .. tostring(vector.cross(V, vector.create(0, 0, 1))))
print("vector-lib2 " .. tostring(vector.floor(vector.create(-1.5, 1.5, 0))) .. " " .. tostring(vector.ceil(vector.create(-1.5, 1.5, 0))) .. " " .. tostring(vector.abs(vector.create(-1, 2, -3))) .. " " .. tostring(vector.sign(vector.create(-2, 0, 3))) .. " " .. tostring(vector.clamp(V, vector.zero, vector.one)) .. " " .. tostring(vector.max(V, vector.create(2, 2, 2))) .. " " .. tostring(vector.min(V, vector.create(2, 2, 2))))
print("vector-angle " .. P.try(function()
	return P.f32(vector.angle(vector.create(1, 0, 0), vector.create(0, 1, 0)))
end) .. " lerp " .. P.try(function()
	return tostring(vector.lerp(vector.zero, V, 0.5))
end))
print("vector-normalize-zero " .. tostring(vector.normalize(vector.zero)))
print("vector-precision " .. P.f32(vector.create(0.1, 0, 0).x) .. " " .. tostring(vector.create(0.1, 0.2, 0.3)) .. " " .. tostring(vector.create(1e10, -1e-10, 1 / 3)))
print("vector-vs-Vector3 " .. tostring(V == Vector3.new(1, 2, 3)) .. " " .. typeof(Vector3.new(1, 2, 3)) .. " " .. tostring(Vector3.new(1, 2, 3)) .. " " .. tostring(rawequal(V, Vector3.new(1, 2, 3))))
print("vector-index " .. P.try(function()
	return V.w
end) .. " | " .. P.try(function()
	return V.Magnitude
end) .. " | " .. P.try(function()
	return V.Unit
end))
print("vector-key " .. (function()
	local Map = {}
	Map[vector.create(1, 2, 3)] = "a"
	return tostring(Map[vector.create(1, 2, 3)]) .. " " .. tostring(Map[Vector3.new(1, 2, 3)])
end)())
print("vector-create-2 " .. P.try(function()
	return tostring(vector.create(1, 2))
end) .. " | " .. P.try(function()
	return tostring(vector.create(1, 2, 3, 4))
end))
print("vector-constants " .. tostring(vector.zero) .. " " .. tostring(vector.one))
print("vector-format " .. P.try(string.format, "%s", V) .. " " .. P.try(string.format, "%*", V) .. " concat " .. P.try(function()
	return "v=" .. V
end))

-- os and DateTime at fixed instants
local Instants = { 0, 86399, 951782400, 1700000000, 1790460000.5, -86400, 2147483648, 4102444800, 253402300799 }
for _, Instant in Instants do
	local Ok, Text = pcall(os.date, "!%Y-%m-%d %H:%M:%S %j %a %A %b %B %p %y %w %%", Instant)
	print("os.date " .. P.num(Instant) .. " " .. (if Ok then tostring(Text) else "E(" .. P.err(Text) .. ")"))
end
local Fields = os.date("!*t", 1700000000)
local Keys = {}
for Key, Value in Fields do
	table.insert(Keys, Key .. "=" .. tostring(Value))
end
table.sort(Keys)
print("os.date-table " .. table.concat(Keys, ","))
print("os.date-default-type " .. type(os.date()) .. " c " .. P.try(os.date, "!%c", 0) .. " x " .. P.try(os.date, "!%x", 0) .. " X " .. P.try(os.date, "!%X", 0))
print("os.date-bad " .. P.try(os.date, "!%Q", 0) .. " | " .. P.try(os.date, "!%Y", "x"))
print("os.time " .. P.try(os.time, { year = 2020, month = 1, day = 1, hour = 0 }) .. " | " .. P.try(os.time, { year = 2020, month = 13, day = 40 }) .. " | " .. P.try(os.time, { year = 2020 }) .. " | type " .. type(os.time()) .. " integer " .. tostring(os.time() % 1 == 0))
print("os.difftime " .. P.try(os.difftime, 10, 4) .. " clock-type " .. type(os.clock()))
print("os.other " .. tostring(os.getenv) .. " " .. tostring(os.exit) .. " " .. tostring(os.execute) .. " " .. tostring(os.remove))
for _, Instant in { 0, 1700000000, 1700000000.123, -1, 253402300799 } do
	local Ok, Time = pcall(DateTime.fromUnixTimestamp, Instant)
	if not Ok then
		print("DateTime " .. P.num(Instant) .. " E(" .. P.err(Time) .. ")")
		continue
	end
	local Universal = Time:ToUniversalTime()
	print("DateTime " .. P.num(Instant) .. " iso " .. Time:ToIsoDate() .. " ms " .. P.num(Time.UnixTimestampMillis) .. " s " .. P.num(Time.UnixTimestamp) .. " parts " .. table.concat({ Universal.Year, Universal.Month, Universal.Day, Universal.Hour, Universal.Minute, Universal.Second, Universal.Millisecond }, ",") .. " fmt " .. Time:FormatUniversalTime("LL LT dddd MMMM Do YYYY HH:mm:ss.SSS A", "en-us"))
end
print("DateTime-millis " .. DateTime.fromUnixTimestampMillis(1700000000123):ToIsoDate())
print("DateTime-iso-parse " .. P.try(function()
	return DateTime.fromIsoDate("2020-02-29T12:34:56Z").UnixTimestamp
end) .. " | " .. P.try(function()
	return DateTime.fromIsoDate("2020-02-29T12:34:56.789+02:00").UnixTimestampMillis
end) .. " | " .. tostring(DateTime.fromIsoDate("not a date")))
print("DateTime-universal " .. P.try(function()
	return DateTime.fromUniversalTime(2021, 2, 30, 25, 61, 61, 1500).UnixTimestampMillis
end))
print("DateTime-locales " .. P.try(function()
	local Time = DateTime.fromUnixTimestamp(1700000000)
	return Time:FormatUniversalTime("LLLL", "fr-fr") .. " | " .. Time:FormatUniversalTime("LLLL", "ja-jp") .. " | " .. Time:FormatUniversalTime("lll", "de-de")
end))
print("DateTime-tostring " .. tostring(DateTime.fromUnixTimestamp(0)) .. " typeof " .. typeof(DateTime.now()))
