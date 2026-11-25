-- FormatByKey's formats by locale (services/localization.rs): `num` with each locale's separators (a few locales have their own,
-- the rest a default), and the formats that read no local time: int, fixed, hex / HEX (their ranges), datetime and iso8601
-- (numbers rounded half away from zero, their range, ISO texts and their checks) and translate. Short dates and times are the
-- host's local time and are left out. The same on a client and a server.
local function S(v, depth)
	depth = depth or 0
	local t = typeof(v)
	if t == "table" then
		if depth > 5 then return "{...}" end
		local keys = {}
		for k in pairs(v) do table.insert(keys, k) end
		table.sort(keys, function(a, b)
			if type(a) == type(b) and (type(a) == "number" or type(a) == "string") then return a < b end
			return type(a) < type(b)
		end)
		local parts = {}
		for _, k in keys do table.insert(parts, (if type(k) == "string" then k else "[" .. tostring(k) .. "]") .. "=" .. S(v[k], depth + 1)) end
		return "{" .. table.concat(parts, ",") .. "}"
	elseif t == "string" then return string.format("%q", v)
	elseif t == "Instance" then return v.ClassName .. "<" .. v.Name .. ">"
	elseif t == "number" then return P.num(v)
	else return t .. ":" .. tostring(v) end
end
local function T(label, f, ...)
	local r = table.pack(pcall(f, ...))
	local parts = {}
	for i = 2, r.n do parts[#parts+1] = if r[1] then S(r[i]) else S(P.err(r[i])) end
	print(label, if r[1] then "ok" else "ERR", table.concat(parts, " "))
end
local function Try(tr, k, args)
	local ok, r = pcall(tr.FormatByKey, tr, k, args)
	return (if ok then "=" else "!") .. tostring(r)
end
local function Keys(L)
	local Out = {}
	for _, E in L:GetEntries() do
		table.insert(Out, E.Key .. "|" .. E.Source .. "|" .. E.Context)
	end
	return table.concat(Out, " ")
end
local function V(Key, Source, Context)
	return { Key = Key, Source = Source, Context = Context, Values = {} }
end
-- Each locale's number and short date/time formats: the table's source locale is the translator's, so the Source renders
local Locales = {
	"en-us", "es-es", "fr-fr", "de-de", "pt-br", "it-it", "ja-jp", "ko-kr", "zh-cn", "zh-tw", "zh-cjv", "ru-ru", "tr-tr", "id-id", "th-th",
	"vi-vn", "pl-pl", "ar-001", "nl-nl", "sv-se", "da-dk", "nb-no", "fi-fi", "cs-cz", "hu-hu", "ro-ro", "uk-ua", "el-gr", "he-il", "hi-in",
	"ms-my", "fil-ph", "bg-bg", "hr-hr", "sk-sk", "sl-si", "sr-rs", "lt-lt", "lv-lv", "et-ee", "bn-bd", "ka-ge", "kk-kz", "km-kh", "my-mm",
	"sq-al", "bs-ba", "en-gb", "es-mx", "fr-ca", "pt-pt", "en", "es", "fr", "de", "xx", "xx-yy", "ab", "en-xx", "de-at", "de-ch", "zh-hk",
	"es-419", "ar", "ar-sa", "fa-ir", "ur-pk", "sw-ke", "ca-es", "eu-es", "gl-es", "is-is", "ga-ie", "mt-mt", "cy-gb", "af-za", "zu-za",
}
local Numbers = { 1234567.891, -0.5, 1e20, 12, 1234.5, -9876.125 }
local Times = { 1700000000, 1690000000, 1704085200, 0, 978307200 }
for _, Loc in Locales do
	local L = Instance.new("LocalizationTable")
	L.SourceLocaleId = Loc
	L:SetEntries({
		{ Key = "num", Source = "{1:num}", Values = {} }, { Key = "sdt", Source = "{1:shortdatetime}", Values = {} },
		{ Key = "sd", Source = "{1:shortdate}", Values = {} }, { Key = "st", Source = "{1:shorttime}", Values = {} },
		{ Key = "dt", Source = "{1:datetime}", Values = {} }, { Key = "def", Source = "{1}", Values = {} }, { Key = "fix", Source = "{1:fixed}", Values = {} },
	})
	local Tr = L:GetTranslator(Loc)
	local Parts = {}
	for _, N in Numbers do
		local Ok, R = pcall(Tr.FormatByKey, Tr, "num", { N })
		table.insert(Parts, S(R))
	end
	for _, K in { "dt", "def", "fix" } do
		local Ok, R = pcall(Tr.FormatByKey, Tr, K, { 1234.5 })
		table.insert(Parts, K .. "=" .. S(R))
	end
	print(S(Loc), S(Tr.LocaleId), table.concat(Parts, " | "))
end

-- dates
local D = Instance.new("LocalizationTable")
D:SetEntries({
	{ Key = "dt", Source = "{1:datetime}", Values = {} }, { Key = "iso", Source = "{1:iso8601}", Values = {} },
	{ Key = "tr", Source = "[{1:translate}]", Values = { ["es-es"] = "<{1:translate}>" } }, { Key = "Hello", Source = "Hello", Values = { ["es-es"] = "Hola" } },
	{ Source = "Bye", Values = { ["es-es"] = "Adios" } },
	{ Key = "named", Source = "{n:translate} {n:int} {n:datetime}", Values = {} },
})
local Dn = D:GetTranslator("en-us")
local De = D:GetTranslator("es-es")
for _, N in { 2147483647, 2147483648, 4294967296, 4102444800, 32503680000, 32536850000, 253402300799, 253402300800, 1e12, 1e15, 1e17, 1e18, 9.2e18, -0.4, -0.5, -1, 0.5, 1.5, 59.5, -1e20, "1970-01-01T00:00:05Z", "2023-11-14T22:13:20Z", "2023-11-14T22:13:20", "2023-11-14", "2023-11-14T22:13:20.5Z", "2023-11-14T22:13:20+01:00", " 2023-11-14T22:13:20Z", "2023-13-14T22:13:20Z", "2023-02-30T00:00:00Z", "1969-12-31T23:59:59Z", "9999-12-31T23:59:59Z", "2023-11-14T25:13:20Z", "2023-1-4T2:3:4Z", "2023-11-14t22:13:20z", "" } do
	T("dt " .. S(N), function() return Try(Dn, "dt", { N }), Try(Dn, "iso", { N }) end)
end
for _, A in { "Hello", "hello", "Bye", "Nope", 5, true, workspace, "" } do
	T("translate " .. S(A), function() return Try(De, "tr", { A }), Try(Dn, "tr", { A }) end)
end
T("named formats", function() return Try(Dn, "named", { n = 5 }), Try(Dn, "named", { n = "x" }) end)
T("hex range", function()
	local H = Instance.new("LocalizationTable")
	H:SetEntries({ { Key = "h", Source = "{1:hex}", Values = {} } })
	local Th = H:GetTranslator("en-us")
	local Out = {}
	for _, N in { 2 ^ 63, -2 ^ 63, 2 ^ 63 - 1024, -2 ^ 63 - 2048, 2 ^ 64, -1e20, -0.9, 0.9, -1.5 } do
		table.insert(Out, Try(Th, "h", { N }))
	end
	return table.concat(Out, " ")
end)
T("int and fixed odd", function()
	local H = Instance.new("LocalizationTable")
	H:SetEntries({ { Key = "i", Source = "{1:int}|{1:fixed}|{1}|{1:num}", Values = {} } })
	local Th = H:GetTranslator("en-us")
	local Out = {}
	for _, N in { -math.huge, 1e300, 5e-324, -1e-9, 0.125, 0.375, 1.125, -2.5, false, 123456789012345678 } do
		table.insert(Out, Try(Th, "i", { N }))
	end
	return table.concat(Out, " ")
end)

-- more locales (a table whose source locale is the translator's renders its Source)
for _, Loc in { "pt", "it", "ja", "ru", "zh", "zh-sg", "zh-mo", "zh-hant", "zh-hans", "es-ar", "es-us", "en-au", "en-ca", "en-in", "pt-ao", "fr-be", "fr-ch", "it-ch", "ar-eg", "ja-xx", "ru-ua", "ko", "de-xx", "en-gb-x", "es-mx-x", "fr-ca ", "EN-GB" } do
	local L = Instance.new("LocalizationTable")
	L.SourceLocaleId = Loc
	L:SetEntries({ { Key = "f", Source = "{1:num}", Values = {} } })
	print("fmt " .. string.format("%q", Loc), Try(L:GetTranslator(Loc), "f", { 1234567.891, 1690000000 }))
end

-- hex
local D = Instance.new("LocalizationTable")
D:SetEntries({ { Key = "h", Source = "{1:hex}", Values = {} } })
local Dn = D:GetTranslator("en-us")
for _, N in { 1.5e19, 1.8e19, 2 ^ 64, 2 ^ 64 + 4096, 1.9e19, 9.3e18, 1e19, -9.3e18 } do
	print("hex " .. string.format("%.17g", N), Try(Dn, "h", { N }))
end
