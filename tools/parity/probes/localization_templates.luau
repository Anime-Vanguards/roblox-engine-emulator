-- FormatByKey's templates (services/localization.rs): which Sources SetEntries takes and its errors for the rest, what each
-- method that takes a text checks (and with which message), `{}` alone, and what FormatByKey writes for every kind of argument
-- (`{1}` from an array, `{name}` from a dictionary, the formats int, fixed, num, hex, HEX, datetime, iso8601, translate) and its
-- errors. The same on a client and a server.
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
-- Which Source texts SetEntries takes, and what FormatByKey makes of the ones it takes
local Sources = {
	"plain", "{}", "{ }", "{1}", "{0}", "{-1}", "{1.5}", "{01}", "{name}", "{Name}", "{na me}", "{na-me}", "{na_me}", "{1name}",
	"{name1}", "{ name }", "{{", "}}", "{{x}}", "{", "}", "a{b", "a}b", "{1:int}", "{1:fixed}", "{1:num}", "{1:HEX}", "{1:hex}",
	"{1:datetime}", "{1:iso8601}", "{1:translate}", "{1:bogus}", "{name:int}", "{1:}", "{:int}", "{1:int:x}",
	"{\u{1F600}}", "{\u{E9}}", "{" .. string.rep("n", 60) .. "}", "{1}{2}{1}", "{2}", "{10}", "x{{1}}y", "{{{1}}}", "{1 }", "{ 1}",
	"{1:INT}", "{1: int}", "{1:int }", "{1:fixed:2}", "{1:%d}", "{1:time}", "{1:date}",
}
local Args = {
	{ "none", nil }, { "empty", {} }, { "str", { "Ann" } }, { "int", { 5 } }, { "neg", { -3 } }, { "frac", { 255.5 } }, { "big", { 1e20 } },
	{ "small", { 0.1 + 0.2 } }, { "bool", { true } }, { "inst", { workspace } }, { "vec", { Vector3.new(1, 2, 3) } },
	{ "named", { name = "Nm", Name = "NM", na_me = "u" } }, { "two", { "a", "b" } }, { "time", { 1700000000 } },
	{ "neg0", { -0.0 } }, { "nan", { 0 / 0 } }, { "inf", { math.huge } }, { "huge int", { 2 ^ 53 } }, { "half", { 2.5 } },
	{ "1.005", { 1.005 } }, { "e-7", { 1.5e-7 } }, { "enum", { Enum.Material.Plastic } }, { "nil hole", { nil, "b" } },
	{ "mixed", { "p", name = "n" } }, { "numstr", { "12" } }, { "color", { Color3.new(1, 0, 0) } }, { "table", { {} } },
}
for _, Source in Sources do
	local L = Instance.new("LocalizationTable")
	local Ok, Problem = pcall(function()
		L:SetEntries({ { Key = "t", Source = Source, Values = { ["es-es"] = Source } } })
	end)
	print("source " .. S(Source), if Ok then "ok" else "ERR " .. S(Problem))
	if Ok then
		local Tr = L:GetTranslator("es-es")
		local Parts = {}
		for _, A in Args do
			local OkF, Result = pcall(Tr.FormatByKey, Tr, "t", A[2])
			table.insert(Parts, A[1] .. (if OkF then "=" else "!") .. S(Result))
		end
		print("  " .. table.concat(Parts, " | "))
	end
end

-- texts in Values, SetEntry and SetEntryValue are checked too?
local L = Instance.new("LocalizationTable")
T("value bad token", function() L:SetEntries({ { Key = "v", Source = "ok", Values = { ["es-es"] = "{}" } } }) return L:GetEntries() end)
T("SetEntry bad token", function() L:SetEntry("w", "es-es", "{}") return L:GetString("es-es", "w") end)
T("SetEntryValue bad token", function() L:SetEntryValue("x", "src", "", "es-es", "{ }") return L:GetString("es-es", "x") end)
T("SetEntrySource bad token", function() L:SetEntrySource("x", "src", "", "{}") return L:GetEntries() end)
T("SetEntryValue bad source", function() L:SetEntryValue("y", "{}", "", "es-es", "v") return L:GetEntries() end)
T("format bad value", function() local Tr = L:GetTranslator("es-es") return Tr:FormatByKey("w") end)
T("format bad value 2", function() local Tr = L:GetTranslator("es-es") return Tr:FormatByKey("w", { "a" }) end)
T("Key with braces", function() local L2 = Instance.new("LocalizationTable") L2:SetEntries({ { Key = "{}", Source = "s", Values = {} } }) return L2:GetEntries() end)
T("Context with braces", function() local L2 = Instance.new("LocalizationTable") L2:SetEntries({ { Key = "k", Source = "s", Context = "{}", Values = {} } }) return L2:GetEntries() end)

-- empty tokens
for _, Source in { "{}", "a{}b", "{}{}", "{1}{}", "{}{1}", "{name}{}", "{{}}", "{}}", "{{}", " {} ", "{}x{}", "x{}", "{} {1}", "{1}{2}", "{name}{1}", "{a}{b}{a}" } do
	local L = Instance.new("LocalizationTable")
	local Ok, Problem = pcall(function() L:SetEntries({ { Key = "t", Source = Source, Values = { ["es-es"] = Source } } }) end)
	if Ok then
		local E = L:GetEntries()[1]
		local Tr = L:GetTranslator("es-es")
		local En = L:GetTranslator("en-us")
		print("src " .. S(Source), "stored", S(E.Source), S(E.Values["es-es"]), "fmt", S({ pcall(Tr.FormatByKey, Tr, "t", { "A", "B" }) }), S({ pcall(En.FormatByKey, En, "t", { a = "x", b = "y", name = "n" }) }))
	else
		print("src " .. S(Source), "ERR", S(Problem))
	end
end
-- the same texts through the other methods
for _, Text in { "{ }", "{", "}", "a{b", "{1:bogus}", "{0}", "{1} {name}", "{}" } do
	local L = Instance.new("LocalizationTable")
	T("values " .. S(Text), function() L:SetEntries({ { Key = "t", Source = "s", Values = { ["es-es"] = Text } } }) return L:GetEntries()[1].Values["es-es"] end)
	T("SetEntry " .. S(Text), function() L:SetEntry("u", "es-es", Text) return L:GetString("es-es", "u") end)
	T("SetEntryValue " .. S(Text), function() L:SetEntryValue("v", "s2", "", "es-es", Text) return L:GetString("es-es", "v") end)
	T("SetEntryValue src " .. S(Text), function() L:SetEntryValue("w", Text, "", "es-es", "x") return Keys(L) end)
	T("SetEntrySource " .. S(Text), function() L:SetEntrySource("v", "s2", "", Text) return Keys(L) end)
	T("SetContents " .. S(Text), function() L:SetContents('[{"key":"c","source":' .. game:GetService("HttpService"):JSONEncode(Text) .. ',"values":{}}]') return Keys(L) end)
end
T("named after numeric", function() Instance.new("LocalizationTable"):SetEntries({ V("k", "{name} {1}", "") }) end)

-- a stored "{}", and a bad template that SetEntries took
local B = Instance.new("LocalizationTable")
B:SetEntries({ { Key = "e", Source = "{}", Values = { ["es-es"] = "{}" } }, { Key = "b", Source = "ok", Values = { ["es-es"] = "{ }", ["fr-fr"] = "{" } } })
print("contents", B:GetContents())
print("bad value", Try(B:GetTranslator("es-es"), "b"), Try(B:GetTranslator("fr-fr"), "b"), Try(B:GetTranslator("es-es"), "e"))
print("bad translate", B:GetTranslator("es-es"):Translate(game, "ok"))
print("GetString {}", B:GetString("es-es", "e"))
