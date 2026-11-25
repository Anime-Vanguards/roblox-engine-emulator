-- LocalizationTable's entries (services/localization.rs): SetEntries' checks and errors, the order GetEntries lists entries in
-- (grouped by Source, the groups in the order they were made, each by Context then Key), the entry methods (made, found by Key or
-- by Source and Context, refiled, conflicts, entries lost), locale ids, and the contents' JSON (GetContents, SetContents' errors,
-- a copy). The same on a client and a server.
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
local LT = Instance.new("LocalizationTable")
print("defaults", LT.SourceLocaleId, LT.DevelopmentLanguage, LT.Name, S(LT.Root))
T("GetEntries empty", function() return LT:GetEntries() end)
T("GetContents empty", function() return LT:GetContents() end)
T("Contents read", function() return LT.Contents end)
local entries = {
	{ Key = "Greeting", Source = "Hello", Context = "", Example = "", Values = { ["es-es"] = "Hola", ["fr-fr"] = "Bonjour" } },
	{ Key = "Named", Source = "Hi {name}, you have {count} coins", Values = { ["es-es"] = "Hola {name}, tienes {count} monedas" } },
	{ Key = "Pos", Source = "{1} and {2}", Values = { ["es-es"] = "{2} y {1}" } },
	{ Key = "", Source = "Keyless source", Values = { ["es-es"] = "Fuente sin clave" } },
	{ Source = "NoKeyField", Values = {} },
	{ Key = "Alpha", Source = "A", Context = "Ctx", Example = "ex", Values = { ["en-us"] = "A-us", ["de-de"] = "A-de" } },
	{ Key = "Zed", Source = "Z" },
	{ Key = "Beta", Source = "B", Values = { ["es-es"] = "" } },
}
T("SetEntries", function() LT:SetEntries(entries) end)

T("GetEntries", function() return LT:GetEntries() end)
T("GetContents", function() return LT:GetContents() end)
T("same table twice", function() return LT:GetEntries() == LT:GetEntries() end)
T("mutating input after", function() entries[1].Source = "Changed"; return LT:GetEntries()[1] end)

-- bad SetEntries input
local bads = {
	{ "nil", nil }, { "number", 5 }, { "string", "x" }, { "bool", true }, { "instance", LT }, { "empty", {} },
	{ "list of numbers", { 5 } }, { "list of empty", { {} } }, { "key number", { { Key = 5, Source = "x" } } },
	{ "source number", { { Key = "a", Source = 5 } } }, { "values number", { { Key = "a", Source = "s", Values = 5 } } },
	{ "values array", { { Key = "a", Source = "s", Values = { "x" } } } }, { "values num value", { { Key = "a", Source = "s", Values = { ["es-es"] = 5 } } } },
	{ "dup keys", { { Key = "a", Source = "x" }, { Key = "a", Source = "y" } } },
	{ "dup source no key", { { Source = "x" }, { Source = "x" } } },
	{ "dup source ctx", { { Source = "x", Context = "c1" }, { Source = "x", Context = "c2" } } },
	{ "bad locale", { { Key = "a", Source = "s", Values = { ["xx"] = "v", ["EN-US"] = "w", ["es_es"] = "z" } } } },
	{ "extra fields", { { Key = "a", Source = "s", Foo = "bar", Values = {} } } },
	{ "dict top", { x = { Key = "a", Source = "s" } } },
	{ "no source", { { Key = "a" } } },
	{ "context number", { { Key = "a", Source = "s", Context = 5 } } },
	{ "example bool", { { Key = "a", Source = "s", Example = true } } },
	{ "lowercase fields", { { key = "a", source = "s", values = { ["es-es"] = "v" } } } },
}
for _, b in bads do
	local L = Instance.new("LocalizationTable")
	T("SetEntries " .. b[1], function() L:SetEntries(b[2]) return L:GetEntries() end)

end

local L = Instance.new("LocalizationTable")
local function E(label) T("  entries " .. label, function() return L:GetEntries() end) end
T("SetEntry", function() L:SetEntry("K1", "es-es", "Uno") end) E("SetEntry")
T("SetEntry second locale", function() L:SetEntry("K1", "fr-fr", "Un") end) E("2")
T("SetEntry source locale", function() L:SetEntry("K1", "en-us", "One") end) E("3")
T("SetEntry bad locale", function() L:SetEntry("K1", "xx", "?") end) E("4")
T("SetEntry empty key", function() L:SetEntry("", "es-es", "vacio") end) E("5")
T("SetEntry nil text", function() L:SetEntry("K2", "es-es") end) E("6")
T("SetEntry nil key", function() L:SetEntry(nil, "es-es", "x") end)
T("SetEntry num key", function() L:SetEntry(5, "es-es", "cinco") end) E("7")

T("GetString es K1", function() return L:GetString("es-es", "K1") end)
T("GetString en K1", function() return L:GetString("en-us", "K1") end)
T("GetString de K1", function() return L:GetString("de-de", "K1") end)
T("GetString es missing", function() return L:GetString("es-es", "Nope") end)
T("GetString bad locale", function() return L:GetString("xx", "K1") end)
T("GetString nil", function() return L:GetString() end)

T("SetEntryValue new", function() L:SetEntryValue("K3", "Three", "", "es-es", "Tres") end) E("sev1")
T("SetEntryValue existing by key", function() L:SetEntryValue("K3", "Three", "", "fr-fr", "Trois") end) E("sev2")
T("SetEntryValue mismatched source", function() L:SetEntryValue("K3", "Other", "", "de-de", "Drei") end) E("sev3")
T("SetEntryValue no key", function() L:SetEntryValue("", "Free", "", "es-es", "Libre") end) E("sev4")
T("SetEntryValue no key ctx", function() L:SetEntryValue("", "Free", "c", "es-es", "Libre c") end) E("sev5")
T("SetEntryValue bad locale", function() L:SetEntryValue("K3", "Three", "", "zz", "?") end) E("sev6")

T("SetEntryContext", function() L:SetEntryContext("K3", "Three", "", "NewCtx") end) E("ctx")
T("SetEntryContext missing", function() L:SetEntryContext("Nope", "", "", "X") end)
T("SetEntryExample", function() L:SetEntryExample("K3", "Three", "NewCtx", "an example") end) E("ex")
T("SetEntrySource", function() L:SetEntrySource("K3", "Three", "NewCtx", "Tri") end) E("src")
T("SetEntryKey", function() L:SetEntryKey("K3", "Tri", "NewCtx", "K3b") end) E("key")
T("SetEntryKey to existing", function() L:SetEntryKey("K3b", "Tri", "NewCtx", "K1") end) E("key2")
T("SetEntryKey missing", function() L:SetEntryKey("Nope", "", "", "X") end)

T("RemoveEntryValue", function() L:RemoveEntryValue("K1", "", "", "fr-fr") end) E("rev")
T("RemoveEntryValue missing", function() L:RemoveEntryValue("Nope", "", "", "fr-fr") end)
T("RemoveEntryValue missing locale", function() L:RemoveEntryValue("K1", "", "", "it-it") end)
T("RemoveTargetLocale", function() L:RemoveTargetLocale("es-es") end) E("rtl")
T("RemoveTargetLocale missing", function() L:RemoveTargetLocale("ja-jp") end)
T("RemoveTargetLocale bad", function() L:RemoveTargetLocale("xx") end)
T("RemoveKey", function() L:RemoveKey("K1") end) E("rk")
T("RemoveKey missing", function() L:RemoveKey("Nope") end)
T("RemoveEntry", function() L:RemoveEntry("K3b", "Tri", "NewCtx") end) E("re")
T("RemoveEntry free", function() L:RemoveEntry("", "Free", "") end) E("re2")
T("RemoveEntry missing", function() L:RemoveEntry("Nope", "x", "y") end)

-- entries made by key alone and found by source
local M = Instance.new("LocalizationTable")
M:SetEntries({ { Key = "k", Source = "S", Context = "C", Values = { ["es-es"] = "v" } } })
T("RemoveEntry by key only", function() M:RemoveEntry("k", "", "") return M:GetEntries() end)
M:SetEntries({ { Key = "k", Source = "S", Context = "C", Values = { ["es-es"] = "v" } } })
T("RemoveEntry by source ctx", function() M:RemoveEntry("", "S", "C") return M:GetEntries() end)
M:SetEntries({ { Key = "k", Source = "S", Context = "C", Values = { ["es-es"] = "v" } } })
T("RemoveEntry wrong ctx", function() M:RemoveEntry("k", "S", "D") return M:GetEntries() end)
M:SetEntries({ { Key = "k", Source = "S", Context = "C", Values = { ["es-es"] = "v" } } })
T("SetEntryValue by key ignores src", function() M:SetEntryValue("k", "", "", "fr-fr", "w") return M:GetEntries() end)
T("SetEntryValue by src ctx", function() M:SetEntryValue("", "S", "C", "de-de", "x") return M:GetEntries() end)

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
-- order: the same entries given in different orders
local Sets = {
	{ V("b", "", ""), V("a", "", ""), V("c", "", "") },
	{ V("c", "", ""), V("b", "", ""), V("a", "", "") },
	{ V("a", "", ""), V("c", "", ""), V("b", "", "") },
	{ V("Zeta", "", ""), V("alpha", "", ""), V("Beta", "", ""), V("10", "", ""), V("9", "", ""), V("_x", "", ""), V("", "free", ""), V("", "Free", ""), V("", "free", "ctx") },
	{ V("", "free", "ctx"), V("", "Free", ""), V("", "free", ""), V("_x", "", ""), V("9", "", ""), V("10", "", ""), V("Beta", "", ""), V("alpha", "", ""), V("Zeta", "", "") },
	{ V("k1", "zz", ""), V("k2", "aa", ""), V("k3", "mm", "c") },
	{ V("k3", "mm", "c"), V("k2", "aa", ""), V("k1", "zz", "") },
	{ V("k1", "", "z"), V("k2", "", "a") },
	{ V("", "S", "C"), V("k", "S", "C") },
	{ V("k", "S", "C"), V("", "S", "C") },
	{ V("", "B", ""), V("", "A", "") },
	{ V("", "A", "y"), V("", "A", "x"), V("", "A", "") },
}
for I, Set in Sets do
	local L = Instance.new("LocalizationTable")
	T("order " .. I, function() L:SetEntries(Set) return Keys(L) end)
end
local Words = { "apple", "Banana", "cherry", "date", "Elder", "fig", "grape", "Hazel", "iris", "jade", "kiwi", "lemon", "Mango", "nut", "olive", "pear", "quince", "rose", "Sage", "thyme" }
local L = Instance.new("LocalizationTable")
for _, W in Words do
	L:SetEntry(W, "es-es", W)
end
T("SetEntry order fwd", function() return Keys(L) end)
local L2 = Instance.new("LocalizationTable")
for I = #Words, 1, -1 do
	L2:SetEntry(Words[I], "es-es", Words[I])
end
T("SetEntry order rev", function() return Keys(L2) end)
local Big = {}
for I = 1, 30 do
	table.insert(Big, V("key" .. I, "", ""))
end
local L3 = Instance.new("LocalizationTable")
L3:SetEntries(Big)
T("order 30", function() return Keys(L3) end)
-- duplicates and failure atomicity
for _, Case in {
	{ "dup keys", { { Key = "a", Source = "x", Values = { ["es-es"] = "1" } }, { Key = "a", Source = "y", Values = { ["fr-fr"] = "2" } } } },
	{ "dup source no key", { { Source = "x", Values = { ["es-es"] = "1" } }, { Source = "x", Values = { ["fr-fr"] = "2" } } } },
	{ "dup source ctx", { { Source = "x", Context = "c1", Values = {} }, { Source = "x", Context = "c2", Values = {} } } },
	{ "dup keyed and free", { { Key = "k", Source = "x", Values = {} }, { Source = "x", Values = {} } } },
	{ "empty key empty source", { { Key = "", Source = "", Values = {} } } },
	{ "values mixed", { { Key = "a", Values = { [1] = "x", ["es-es"] = "y" } } } },
	{ "values empty locale", { { Key = "a", Values = { [""] = "y" } } } },
	{ "values number key", { { Key = "a", Values = { [5] = "y" } } } },
	{ "values bool", { { Key = "a", Values = true } } },
	{ "entry with array part", { { "x", Key = "a", Values = {} } } },
	{ "sparse", { [1] = V("a", "", ""), [3] = V("c", "", "") } },
	{ "second bad", { V("a", "", ""), 5 } },
	{ "values case", { { Key = "a", Values = { ["Es-ES"] = "x", ["es-es"] = "y" } } } },
	{ "values long locale", { { Key = "a", Values = { ["abcdefghijklmnopqrstuvwxyz"] = "y", ["e"] = "z", ["en-us-x"] = "w" } } } },
	{ "key with spaces", { { Key = " a ", Source = " s ", Context = " c ", Example = " e ", Values = {} } } },
} do
	local L4 = Instance.new("LocalizationTable")
	L4:SetEntries({ V("keep", "", "") })
	T("SetEntries " .. Case[1], function() L4:SetEntries(Case[2]) end)
	T("  after", function() return L4:GetEntries() end)

end
-- GetContents
local L5 = Instance.new("LocalizationTable")
L5:SetEntries({ { Key = "k", Source = "s", Context = "c", Example = "e", Values = { ["es-es"] = "v", ["fr-fr"] = "w" } }, { Source = "free", Values = {} }, { Key = "q\"uote\n", Source = "é", Values = { ["es-es"] = "ü\t" } } })
T("GetContents", function() return L5:GetContents() end)
T("SetContents back", function() local L6 = Instance.new("LocalizationTable") L6:SetContents(L5:GetContents()) return L6:GetEntries() end)
T("SetContents bad", function() local L6 = Instance.new("LocalizationTable") L6:SetContents("not json") return L6:GetEntries() end)
T("SetContents nil", function() local L6 = Instance.new("LocalizationTable") L6:SetContents() return L6:GetEntries() end)
T("SetContents capital", function() local L6 = Instance.new("LocalizationTable") L6:SetContents('[{"Key":"k","Source":"s","Values":{"es-es":"v"}}]') return L6:GetEntries() end)

-- SourceLocaleId writes
for _, Value in { "es-es", "", "xx", "EN-US", "es_ES", 5 } do
	local L7 = Instance.new("LocalizationTable")
	T("SourceLocaleId=" .. tostring(Value), function() L7.SourceLocaleId = Value return L7.SourceLocaleId, L7.DevelopmentLanguage end)

end
T("DevelopmentLanguage write", function() local L8 = Instance.new("LocalizationTable") L8.DevelopmentLanguage = "fr-fr" return L8.SourceLocaleId end)
-- Clone, entry tables
local C = L5:Clone()
T("clone entries", function() return #C:GetEntries(), C:GetContents() == L5:GetContents() end)
local Got = L5:GetEntries()
Got[1].Key = "changed"
T("returned copies", function() return L5:GetEntries()[1].Key end)
T("entry table frozen", function() return table.isfrozen(Got), table.isfrozen(Got[1]), getmetatable(Got) end)
-- argument casts
local L9 = Instance.new("LocalizationTable")
T("SetEntry casts", function() L9:SetEntry(1, 2, 3) return L9:GetEntries() end)
T("SetEntry table", function() L9:SetEntry({}, "es-es", "x") return L9:GetEntries() end)
T("GetString casts", function() return L9:GetString(2, 1) end)
T("GetTranslator table", function() return L9:GetTranslator({}).LocaleId end)
T("GetTranslator EN-US", function() return L9:GetTranslator("EN-US").LocaleId, L9:GetTranslator("es_ES").LocaleId end)
T("GetTranslator 1 char", function() return L9:GetTranslator("e").LocaleId end)
T("GetTranslator long", function() return L9:GetTranslator("abcdefghijklmnopqrstuvwxyz").LocaleId end)
T("dot call", function() return L9.GetEntries(workspace) end)

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
-- 30 sources in one order and in reverse (insertion order, or scrambled?)
local Sources = {}
for I = 1, 30 do
	table.insert(Sources, V("k" .. I, "src" .. I, ""))
end
local L = Instance.new("LocalizationTable")
L:SetEntries(Sources)
T("30 sources fwd", function() return Keys(L) end)
local Rev = {}
for I = 30, 1, -1 do
	table.insert(Rev, Sources[I])
end
L:SetEntries(Rev)
T("30 sources rev", function() return Keys(L) end)
local Words = { "pear", "Apple", "zebra", "mango", "", "kiwi", "Banana", "fig", "date", "cherry" }
local W = {}
for I, Word in Words do
	table.insert(W, V("w" .. I, Word, ""))
end
L:SetEntries(W)
T("words as sources", function() return Keys(L) end)
-- source changes, removals and re-adds
local M = Instance.new("LocalizationTable")
M:SetEntries({ V("a", "S1", ""), V("b", "S2", ""), V("c", "S3", "") })
M:SetEntrySource("a", "S1", "", "S9")
T("after source a->S9", function() return Keys(M) end)
M:SetEntryValue("d", "S1", "", "es-es", "v")
T("add d with S1", function() return Keys(M) end)
M:SetEntryValue("e", "S9", "", "es-es", "v")
T("add e with S9", function() return Keys(M) end)
M:RemoveKey("b")
T("remove b", function() return Keys(M) end)
M:SetEntryValue("f", "S2", "", "es-es", "v")
T("add f with S2", function() return Keys(M) end)
M:SetEntryValue("g", "S0", "", "es-es", "v")
T("add g with S0", function() return Keys(M) end)

-- regrouping
local M = Instance.new("LocalizationTable")
M:SetEntries({ V("a", "S1", ""), V("b", "S1", ""), V("c", "S2", "") })
M:SetEntrySource("a", "S1", "", "S9")
T("a out of a shared group", function() return Keys(M) end)
M:SetEntrySource("c", "S2", "", "S1")
T("c into S1", function() return Keys(M) end)
M:SetEntryValue("d", "S2", "", "es-es", "v")
T("d into S2", function() return Keys(M) end)
M:SetEntrySource("b", "S1", "", "S8")
T("b out", function() return Keys(M) end)
M:SetEntryValue("e", "S1", "", "es-es", "v")
T("e into S1", function() return Keys(M) end)
local F = Instance.new("LocalizationTable")
F:SetEntries({ V("", "S1", ""), V("x", "S2", "") })
F:SetEntrySource("", "S1", "", "S3")
T("free source change", function() return Keys(F) end)
F:SetEntryValue("y", "S1", "", "es-es", "v")
T("y into S1", function() return Keys(F) end)
-- failures part way
local G = Instance.new("LocalizationTable")
G:SetEntries({ V("a", "", ""), V("c", "", "") })
T("SetEntryKey to empty", function() G:SetEntryKey("c", "", "", "") end)
T("  after", function() return Keys(G) end)
T("SetEntrySource to empty", function() G:SetEntries({ V("", "S", "") }) G:SetEntrySource("", "S", "", "") end)
T("  after", function() return Keys(G) end)
T("SetEntrySource conflict", function() G:SetEntries({ V("", "A", ""), V("", "B", "") }) G:SetEntrySource("", "A", "", "B") end)
T("  after", function() return Keys(G) end)
T("SetEntryContext conflict", function() G:SetEntries({ V("", "A", "x"), V("", "A", "y") }) G:SetEntryContext("", "A", "x", "y") end)
T("  after", function() return Keys(G) end)
T("SetEntryKey missing", function() G:SetEntries({}) G:SetEntryKey("m", "", "", "made") return Keys(G) end)
T("SetEntrySource missing", function() G:SetEntries({}) G:SetEntrySource("m", "", "", "Made") return Keys(G) end)
T("SetEntryExample missing", function() G:SetEntries({}) G:SetEntryExample("m", "", "", "ex") return G:GetEntries() end)
T("SetEntryValue empty", function() G:SetEntries({}) G:SetEntryValue("", "", "", "es-es", "x") return Keys(G) end)
T("SetEntryContext free missing", function() G:SetEntries({}) G:SetEntryContext("", "Free", "", "c") return Keys(G) end)
T("RemoveEntry empty", function() G:SetEntries({ V("k", "", "") }) G:RemoveEntry("", "", "") return Keys(G) end)
T("SetEntryValue empty locale", function() G:SetEntries({ V("k", "", "") }) G:SetEntryValue("k", "", "", "", "x") return G:GetEntries() end)

-- contents
local C = Instance.new("LocalizationTable")
C:SetEntries({ { Key = "k", Source = "s", Values = { ["zz"] = "1", ["aa"] = "2", ["mm"] = "3", ["es-es"] = "4", ["ES"] = "5" } } })
T("values order", function() return C:GetContents() end)
T("keyed no source", function() local C3 = Instance.new("LocalizationTable") C3:SetEntries({ { Key = "k", Values = {} } }) return C3:GetContents() end)
for _, Case in {
	{ ["ES-ES"] = "x", ["es-es"] = "y" }, { ["es-es"] = "y", ["ES-ES"] = "x" }, { ["es_es"] = "a", ["es-es"] = "b" }, { ["es-es"] = "b", ["es_es"] = "a" },
	{ ["Es_Es"] = "1", ["eS-eS"] = "2", ["es-es"] = "3" },
} do
	local C2 = Instance.new("LocalizationTable")
	T("collide", function() C2:SetEntries({ { Key = "k", Values = Case } }) return C2:GetEntries()[1].Values end)
end
T("SetEntry locale case", function() local C3 = Instance.new("LocalizationTable") C3:SetEntry("k", "ES_ES", "v") return C3:GetEntries() end)
T("SetEntryValue locale case", function() local C3 = Instance.new("LocalizationTable") C3:SetEntryValue("k", "", "", "FR_fr", "v") return C3:GetEntries() end)
T("GetString locale case", function() local C3 = Instance.new("LocalizationTable") C3:SetEntry("k", "es-es", "v") return C3:GetString("ES_ES", "k") end)
T("RemoveEntryValue locale case", function() local C3 = Instance.new("LocalizationTable") C3:SetEntry("k", "es-es", "v") C3:RemoveEntryValue("k", "", "", "ES-ES") return C3:GetEntries() end)
T("RemoveTargetLocale case", function() local C3 = Instance.new("LocalizationTable") C3:SetEntry("k", "es-es", "v") C3:RemoveTargetLocale("ES_ES") return C3:GetEntries() end)
T("SetEntry empty locale", function() local C3 = Instance.new("LocalizationTable") C3:SetEntry("k", "", "v") return C3:GetEntries() end)
T("SetContents example", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"key":"k","source":"s","example":"e1","examples":"e2","context":"c","values":{"es-es":"v"}}]') return C3:GetEntries() end)
T("SetContents order", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"key":"b","source":"x","values":{}},{"key":"a","source":"x","values":{}},{"key":"c","source":"","values":{}}]') return Keys(C3) end)
T("SetContents dup", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"key":"a","source":"x","values":{}},{"key":"a","source":"y","values":{}}]') return Keys(C3) end)
T("SetContents dup free", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"source":"x","values":{"es-es":"1"}},{"source":"x","values":{"fr-fr":"2"}}]') return C3:GetEntries() end)
T("SetContents bad entry", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"source":"","values":{}}]') return Keys(C3) end)
T("SetContents object", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('{}') return Keys(C3) end)
T("SetContents number value", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"key":"k","values":{"es-es":5}}]') return C3:GetEntries() end)
T("SetContents number key", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"key":5,"source":"s","values":{}}]') return C3:GetEntries() end)
T("SetContents not object", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[5]') return Keys(C3) end)
T("SetContents values array", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"key":"k","values":[]}]') return Keys(C3) end)
T("SetContents empty", function() local C3 = Instance.new("LocalizationTable") C3:SetEntries({ { Key = "keep", Values = {} } }) C3:SetContents("") return Keys(C3) end)
T("SetContents keeps on error", function() local C3 = Instance.new("LocalizationTable") C3:SetEntries({ { Key = "keep", Values = {} } }) pcall(C3.SetContents, C3, "[{\"key\":\"x\"}]") return Keys(C3) end)
T("SetContents upper locale", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"key":"k","values":{"ES_ES":"v","":"e"}}]') return C3:GetEntries() end)
T("SetContents extra", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"key":"k","values":{},"foo":1}]') return C3:GetEntries() end)
T("SetContents trailing", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[] x') return Keys(C3) end)
T("SetContents escapes", function() local C3 = Instance.new("LocalizationTable") C3:SetContents('[{"key":"\\u00e9\\n\\/","values":{}}]') return C3:GetEntries() end)
T("contents escapes", function() local C3 = Instance.new("LocalizationTable") C3:SetEntries({ { Key = "\1\31\127/\\<>&'\u{2028}\u{e9}\8\12\13", Source = "", Values = {} } }) return C3:GetContents() end)
T("clone order", function() local C3 = Instance.new("LocalizationTable") C3:SetEntries({ { Key = "b", Source = "y", Values = {} }, { Key = "a", Source = "x", Values = {} } }) return Keys(C3:Clone()) end)

