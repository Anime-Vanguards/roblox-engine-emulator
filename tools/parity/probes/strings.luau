-- string.format, tostring and tonumber over generated values, and the string library's edge cases.
local Rng = P.rng(12345)
local Specials = { 0, -0, 1, -1, 0.1, 0.5, 1 / 3, 2 / 3, 100, 1e15, 1e16, 1e17, 2 ^ 53, 2 ^ 53 + 2, 2 ^ 63, 2 ^ 64, 1e100, 1e300, 1e-5, 1e-7, 1e-300, 5e-324, 123456789.123, 0.1 + 0.2, 1e21, 1e22, -1e-10, 255, 65535, 4294967295, 4294967296, -2 ^ 31, 2 ^ 31, math.pi, -math.pi, 1.5, 2.5, -2.5, 0.05, 0.15, 0.25, 1234.5, 99.995, 9.995, math.huge, -math.huge, 0 / 0 }
local Generated = {}
for _ = 1, 60 do
	local Exponent = Rng.int(-20, 25)
	local Mantissa = Rng.real(-10, 10)
	table.insert(Generated, Mantissa * 10 ^ Exponent)
end
for _ = 1, 20 do
	table.insert(Generated, Rng.int(-100000, 100000) / Rng.pick({ 1, 2, 4, 8, 10, 100, 1000, 3, 7 }))
end
local Numbers = table.clone(Specials)
for _, Value in Generated do
	table.insert(Numbers, Value)
end

-- tostring of numbers (Luau's shortest round-trip form)
local Lines = {}
for Index, Value in Numbers do
	table.insert(Lines, tostring(Value))
	if #Lines == 16 or Index == #Numbers then
		print("tostring " .. table.concat(Lines, " "))
		table.clear(Lines)
	end
end
print("concat " .. (1 .. "") .. " " .. (1.5 .. "") .. " " .. (1e100 .. "") .. " " .. (-0 .. "") .. " " .. ((0 / 0) .. "") .. " " .. (math.huge .. ""))

-- string.format specifiers
local Formats = { "%d", "%i", "%5d", "%-5d|", "%05d", "%+d", "%x", "%X", "%#x", "%o", "%c", "%e", "%.3e", "%E", "%f", "%.0f", "%.2f", "%10.3f", "%-10.2f|", "%g", "%.3g", "%10g", "%G", "%a", "%s", "%q", "%5.1s|", "%%" }
for _, Format in Formats do
	local Out = {}
	for _, Value in { 0, -0, 1, -1, 3.5, -2.5, 255.75, 1e10, 1e20, 0.000123456, 2 ^ 53, 2 ^ 63, -2 ^ 63, math.huge, -math.huge, 0 / 0 } do
		local Ok, Text = pcall(string.format, Format, Value)
		table.insert(Out, if Ok then Text else "E(" .. P.err(Text) .. ")")
	end
	print("format " .. Format .. " " .. table.concat(Out, " "))
end
for _, Format in { "%d", "%x", "%5.2f", "%s", "%q", "%c" } do
	local Out = {}
	for _, Value in { "12", "0x10", " 7 ", "abc", true, nil, {}, "\0\1\n\"\\", 65, 2.5 } do
		local Ok, Text = pcall(string.format, Format, Value)
		if Ok and type(Value) == "table" then
			Text = string.gsub(Text, "0x%x+", "0xADDR")
		end
		table.insert(Out, if Ok then string.format("%q", Text) else "E(" .. P.err(Text) .. ")")
	end
	print("format-odd " .. Format .. " " .. table.concat(Out, " "))
end
print("format-generated")
for Index = 1, #Generated, 4 do
	local Out = {}
	for Offset = 0, 3 do
		local Value = Generated[Index + Offset]
		if Value then
			table.insert(Out, string.format("%.3f|%g|%.10g|%e|%.14g", Value, Value, Value, Value, Value))
		end
	end
	print("  " .. table.concat(Out, " "))
end
print("format-missing " .. P.try(string.format, "%d %d", 1))
print("format-bad " .. P.try(string.format, "%y", 1))
print("format-toolong " .. P.try(string.format, "%123d", 1))
print("format-star " .. P.try(string.format, "%*d", 5, 1))
print("format-int-from-float " .. P.try(string.format, "%d", 2.5))
print("format-int-from-string " .. P.try(string.format, "%d", "2.5"))
print("format-%s-types " .. P.try(string.format, "%s %s %s", 1, 1.5, true) .. " | " .. P.try(string.format, "%s", nil))
print("format-%* " .. P.try(string.format, "%* %* %* %*", 1, true, nil, "s") .. " | " .. P.try(function()
	return (string.gsub(string.format("%*", {}), "0x%x+", "ADDR"))
end))
print("format-%s-meta " .. P.try(string.format, "%s", setmetatable({}, { __tostring = function()
	return "custom"
end })) .. " " .. P.try(string.format, "%*", setmetatable({}, { __tostring = function()
	return "custom"
end })))
print("format-%s-meta-nonstring " .. P.try(string.format, "%s", setmetatable({}, { __tostring = function()
	return 5
end })))

-- tonumber
local Inputs = { "10", " 10 ", "10 ", "0x10", "0X1f", "-0x10", "1e3", "1E-3", ".5", "5.", "1e", "e1", "0x", "inf", "nan", "-inf", "1_000", "0b101", "  ", "", "1 2", "+5", "--5", "1.5e+2", "0x1p4", "1f", "١", "123456789012345678901234567890", "0.1e999", "4294967296" }
local Out = {}
for _, Input in Inputs do
	table.insert(Out, string.format("%q", Input) .. "=" .. P.num(tonumber(Input)))
end
print("tonumber " .. table.concat(Out, " "))
Out = {}
for _, Pair in { { "ff", 16 }, { "FF", 16 }, { "z", 36 }, { "Z", 36 }, { "777", 8 }, { "102", 2 }, { "-ff", 16 }, { "0x10", 16 }, { " 10", 10 }, { "10", 1 }, { "10", 37 }, { "1.5", 10 } } do
	local Ok, Value = pcall(tonumber, Pair[1], Pair[2])
	table.insert(Out, Pair[1] .. "/" .. Pair[2] .. "=" .. (if Ok then P.num(Value) else "E(" .. P.err(Value) .. ")"))
end
print("tonumber-base " .. table.concat(Out, " "))
print("tonumber-types " .. P.num(tonumber(5)) .. " " .. tostring(tonumber(nil)) .. " " .. tostring(tonumber(true)) .. " " .. P.try(tonumber))

-- the string library
print("split " .. table.concat(string.split("a,b,,c", ","), "|") .. " / " .. #string.split("", ",") .. " / " .. #string.split("abc", "") .. " / " .. table.concat(string.split("a--b--c", "--"), "|") .. " / " .. table.concat(string.split("a b"), "|"))
print("rep " .. string.rep("ab", 3, "-") .. " " .. string.rep("x", 0) .. "|" .. string.rep("x", -1) .. "|")
print("rep-huge " .. P.try(string.rep, "x", 2 ^ 31))
print("sub " .. string.sub("hello", 2, -2) .. " " .. string.sub("hello", -100, 100) .. " " .. string.sub("hello", 0) .. " " .. string.sub("hello", 3, 2) .. "|")
print("byte " .. table.concat({ string.byte("abc", 1, -1) }, ",") .. " " .. select("#", string.byte("")) .. " char " .. string.char(72, 105))
print("char-range " .. P.try(string.char, 256))
print("find " .. table.concat({ tostring(string.find("hello world", "o w")), tostring(string.find("a.b", ".", 1, true)), tostring(string.find("abc", "b", -1)), tostring(string.find("abc", "", 10)) }, " "))
print("match " .. tostring(string.match("key=value", "(%w+)=(%w+)")) .. " " .. tostring(string.match("  trim  ", "^%s*(.-)%s*$")) .. "|")
print("gsub " .. table.concat({ string.gsub("hello world", "o", "0") }, " ") .. " " .. table.concat({ string.gsub("abc", "%w", "%0%0") }, " ") .. " " .. table.concat({ string.gsub("abc", "", "-") }, " "))
print("gsub-bad " .. P.try(string.gsub, "abc", "(", "x"))
print("gsub-repl " .. P.try(string.gsub, "abc", "b", "%2"))
print("gmatch " .. (function()
	local Parts = {}
	for Word in string.gmatch("one two  three", "%a+") do
		table.insert(Parts, Word)
	end
	return table.concat(Parts, "|")
end)())
print("pattern-frontier " .. table.concat({ string.gsub("THE (quick) fox", "%f[%a]%a+", "W") }, " "))
print("pattern-balanced " .. tostring(string.match("x(a(b)c)y", "%b()")))
print("upper-lower " .. string.upper("abcÄé") .. " " .. string.lower("ABCÄÉ") .. " len " .. #"Ä")
print("reverse " .. string.reverse("abc") .. " format-q-control " .. string.format("%q", "a\r\n\t\0\127\200b"))
print("utf8 " .. utf8.char(72, 228, 8364, 128512) .. " len " .. tostring(utf8.len("Hä€😀")) .. " bad-len " .. table.concat({ tostring(utf8.len("a\255b")) }, ","))
print("utf8-codepoint " .. table.concat({ utf8.codepoint("Hä€", 1, -1) }, ","))
print("utf8-offset " .. tostring(utf8.offset("Hä€😀", 3)) .. " " .. tostring(utf8.offset("Hä€😀", -1)))
print("utf8-charpattern " .. utf8.charpattern)
print("utf8-char-range " .. P.try(utf8.char, -1) .. " " .. P.try(utf8.char, 0x110000))
print("utf8-nfc " .. P.try(function()
	return utf8.nfcnormalize("é")
end))
print("pack " .. P.try(function()
	return #string.pack("<i4d", 1, 2.5)
end) .. " unpack " .. P.try(function()
	return string.unpack("<i4", string.pack("<i4", -7))
end))
print("compare " .. tostring("a" < "b") .. tostring("Z" < "a") .. tostring("" < "a") .. tostring("a\0b" < "a\0c") .. tostring("é" > "z"))
print("len-ops " .. #"" .. " " .. #"\0\0" .. " " .. string.len("abc"))
print("string-index " .. P.try(function()
	return ("abc"):upper()
end) .. " " .. P.try(function()
	return ("abc").len
end))
print("string-arith " .. P.try(function()
	return "10" + 5
end) .. " " .. P.try(function()
	return "0x10" * 2
end) .. " " .. P.try(function()
	return "a" + 1
end) .. " " .. P.try(function()
	return " 3 " * 2
end))
