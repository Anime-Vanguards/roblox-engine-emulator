-- DateTime at fixed instants (universal time only: local time depends on the capturing machine): every format token,
-- fractional and negative timestamps, ISO 8601 parsing, fromUniversalTime's ranges, tostring; and os.date's specifiers.
local Instant = DateTime.fromUnixTimestampMillis(1700000000123)
local Tokens = { "YYYY", "YY", "Y", "Q", "Qo", "M", "Mo", "MM", "MMM", "MMMM", "D", "Do", "DD", "DDD", "DDDo", "DDDD", "d", "do", "dd",
	"ddd", "dddd", "e", "E", "w", "wo", "ww", "W", "Wo", "WW", "gg", "gggg", "GG", "GGGG", "H", "HH", "h", "hh", "k", "kk", "m", "mm", "s",
	"ss", "S", "SS", "SSS", "SSSS", "A", "a", "Z", "ZZ", "X", "x", "LT", "LTS", "L", "l", "LL", "ll", "LLL", "lll", "LLLL", "llll",
	"[text]", "[YYYY]", "YYYY[-]MM", "zz" }
local Parts = {}
for _, Token in Tokens do
	local Ok, Text = pcall(Instant.FormatUniversalTime, Instant, Token, "en-us")
	table.insert(Parts, Token .. "=" .. (if Ok then Text else "E(" .. P.err(Text) .. ")"))
end
print("tokens " .. table.concat(Parts, " | "))
local Morning = DateTime.fromUnixTimestamp(1700020800) -- 2023-11-15T04:00:00Z
print("morning " .. Morning:FormatUniversalTime("h hh H HH k A a LT LTS", "en-us"))
print("midnight " .. DateTime.fromUnixTimestamp(1699920000):FormatUniversalTime("h hh H HH k kk A LT", "en-us"))
print("ordinals " .. (function()
	local Out = {}
	for _, Day in { 1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31 } do
		table.insert(Out, DateTime.fromUniversalTime(2023, 1, Day):FormatUniversalTime("Do", "en-us"))
	end
	return table.concat(Out, ",")
end)())
print("locale-default " .. P.try(Instant.FormatUniversalTime, Instant, "LL", nil) .. " | " .. P.try(Instant.FormatUniversalTime, Instant, "LL", "en-gb")
	.. " | " .. P.try(Instant.FormatUniversalTime, Instant, "LL", "xx-yy") .. " | " .. P.try(Instant.FormatUniversalTime, Instant, "LL", "") .. " | " .. P.try(Instant.FormatUniversalTime, Instant, 5, "en-us"))

-- fractional and negative timestamps
for _, Seconds in { 0.5, 1.999, -0.5, -1.5, 1e-3, 1700000000.9999 } do
	local Ok, Time = pcall(DateTime.fromUnixTimestamp, Seconds)
	print("seconds " .. P.num(Seconds) .. " " .. (if Ok then P.num(Time.UnixTimestampMillis) .. " " .. P.num(Time.UnixTimestamp) .. " " .. tostring(Time) else "E(" .. P.err(Time) .. ")"))
end
for _, Millis in { 1.5, -1.5, 0.4, 1700000000123.7, -0.5 } do
	local Ok, Time = pcall(DateTime.fromUnixTimestampMillis, Millis)
	print("millis " .. P.num(Millis) .. " " .. (if Ok then P.num(Time.UnixTimestampMillis) .. " " .. P.num(Time.UnixTimestamp) .. " " .. Time:FormatUniversalTime("YYYY-MM-DD HH:mm:ss.SSS", "en-us") else "E(" .. P.err(Time) .. ")"))
end
local Fields = DateTime.fromUnixTimestampMillis(-1500):ToUniversalTime()
print("negative-fields " .. table.concat({ Fields.Year, Fields.Month, Fields.Day, Fields.Hour, Fields.Minute, Fields.Second, Fields.Millisecond }, ","))
print("range " .. P.try(DateTime.fromUnixTimestamp, -62135596801) .. " | " .. P.try(DateTime.fromUnixTimestamp, 253402300800) .. " | " .. P.try(DateTime.fromUnixTimestamp, 0 / 0)
	.. " | " .. P.try(DateTime.fromUnixTimestamp, math.huge) .. " | " .. P.try(DateTime.fromUnixTimestamp, "12"))

-- ISO 8601
for _, Text in { "2020-02-29T12:34:56Z", "2020-02-29T12:34:56.789Z", "2020-02-29T12:34:56.789+02:00", "2020-02-29T12:34:56-05:30", "2020-02-29T12:34:56+0200",
	"2020-02-29", "2020-02-29T12:34", "2020-02-29t12:34:56z", "2020-02-29 12:34:56Z", "2020-02-30T00:00:00Z", "2020-02-29T24:00:00Z", "2020-02-29T12:34:56.1234567Z",
	"20200229T123456Z", "+002020-02-29T12:34:56Z", "2020-02-29T12:34:60Z", " 2020-02-29T12:34:56Z" } do
	local Ok, Time = pcall(DateTime.fromIsoDate, Text)
	print("iso " .. Text .. " " .. (if not Ok then "E(" .. P.err(Time) .. ")" elseif Time == nil then "nil" else P.num(Time.UnixTimestampMillis)))
end
print("iso-out " .. DateTime.fromUnixTimestampMillis(1700000000123):ToIsoDate() .. " " .. DateTime.fromUnixTimestampMillis(-1):ToIsoDate() .. " " .. P.try(function()
	return DateTime.fromUnixTimestamp(-62135596800):ToIsoDate()
end))

-- fromUniversalTime: defaults, ranges, fractions
print("universal " .. P.try(function() return DateTime.fromUniversalTime(2021).UnixTimestampMillis end) .. " | " .. P.try(function() return DateTime.fromUniversalTime().UnixTimestampMillis end)
	.. " | " .. P.try(function() return DateTime.fromUniversalTime(2021, 2, 28, 23, 59, 59, 999).UnixTimestampMillis end))
for _, Fields in { { 2021, 13, 1 }, { 2021, 2, 29 }, { 2021, 1, 1, 24 }, { 2021, 1, 1, 0, 60 }, { 2021, 1, 1, 0, 0, 60 }, { 2021, 1, 1, 0, 0, 0, 1000 }, { 2021, 0, 1 }, { 2021, 1, 0 },
	{ 2021.5, 1.5, 1.5 }, { -1, 1, 1 }, { 10000, 1, 1 }, { 2021, 1, 1, -1 } } do
	print("universal-fields " .. table.concat(Fields, ",") .. " " .. P.try(function()
		return DateTime.fromUniversalTime(table.unpack(Fields)).UnixTimestampMillis
	end))
end

-- os.date: specifiers MSVC and C99 know (not %z and %Z: they name the capturing machine's time zone)
local Specifiers = {}
for _, Specifier in { "%a", "%A", "%b", "%B", "%c", "%C", "%d", "%D", "%e", "%F", "%g", "%G", "%h", "%H", "%I", "%j", "%m", "%M", "%n", "%p", "%r", "%R", "%S", "%t", "%T", "%u", "%U", "%V", "%w", "%W", "%x", "%X", "%y", "%Y", "%%", "%E", "%Ec", "%Oy", "%#d", "%-d", "%" } do
	local Ok, Text = pcall(os.date, "!" .. Specifier, 1700000000)
	table.insert(Specifiers, Specifier .. "=" .. (if Ok then P.text(Text) else "E(" .. P.err(Text) .. ")"))
end
print("os.date-specifiers " .. table.concat(Specifiers, " | "))
print("os.date-args " .. P.try(os.date, "!%Y", 1700000000.9) .. " | " .. P.try(os.date, "!%Y", -1) .. " | " .. P.try(os.date, "!%Y", "1700000000") .. " | " .. P.try(os.date, 5, 0)
	.. " | " .. P.try(os.date, "!*t", 0 / 0) .. " | " .. P.try(os.date, "!*t", math.huge) .. " | " .. P.try(function()
		return os.date("!*t", 1700000000.75).sec
	end))
print("os.time-args " .. P.try(os.time, { year = 2020, month = 1, day = 1, hour = 0, min = 0, sec = 0.5 }) .. " | " .. P.try(os.time, { year = 2020, month = 1, day = 1.5 })
	.. " | " .. P.try(os.time, { year = "2020", month = 1, day = 1 }) .. " | " .. P.try(os.time, { year = 1969, month = 12, day = 31, hour = 0 }) .. " | " .. P.try(os.time, 5)
	.. " | " .. P.try(os.time, { year = 2020, month = 1, day = 1, isdst = true }))
