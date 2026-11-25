-- Translators (services/localization.rs): what GetTranslator and LocalizationService's translators are (a new "Instance" each
-- call, a locked parent, a read-only LocaleId), the tables each reads (a table's: it and the tables it is in; the service's: the
-- tables in it that only tables hold, deepest first) as they are at each call, which text FormatByKey finds for a locale (its
-- own, its language's, a paired locale's, another of its language's, a Source in the source locale's language), Translate's
-- choice by Context, LocalizationService's methods and errors, and Roblox's own members. The same on a client and a server.
local LS = game:GetService("LocalizationService")
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
LT:SetEntries({
	{ Key = "Greeting", Source = "Hello", Values = { ["es-es"] = "Hola", ["fr-fr"] = "Bonjour" } },
	{ Key = "Named", Source = "Hi {name}, you have {count} coins", Values = { ["es-es"] = "Hola {name}, tienes {count} monedas" } },
	{ Key = "Pos", Source = "{1} and {2}", Values = { ["es-es"] = "{2} y {1}" } },
	{ Source = "NoKey", Values = { ["es-es"] = "SinClave" } },
	{ Key = "Alpha", Source = "A", Context = "Ctx", Values = { ["en-us"] = "A-us", ["de-de"] = "A-de" } },
	{ Key = "Empty", Source = "E", Values = { ["es-es"] = "" } },
	{ Key = "NoSrc", Values = { ["es-es"] = "SoloEs" } },
	{ Key = "Mixed", Source = "{1} {2}", Values = {} },
})
for _, loc in { "es-es", "en-us", "fr-fr", "de-de", "it-it", "xx-yy", "", "ES-ES", "es", "es_es", "zh-cjv", "en", "en-gb", "pt-br" } do
	T("GetTranslator '" .. loc .. "'", function()
		local tr = LT:GetTranslator(loc)
		return tr, tr and tr.LocaleId, tr and tr.Parent, tr and tr.ClassName
	end)
end
T("GetTranslator nil", function() return LT:GetTranslator() end)
T("GetTranslator num", function() return LT:GetTranslator(5).LocaleId end)

T("same instance", function() return LT:GetTranslator("es-es") == LT:GetTranslator("es-es") end)
local es = LT:GetTranslator("es-es")
T("tr props", function() return es.Name, es.Archivable, es:GetFullName(), #es:GetChildren() end)
T("tr LocaleId write", function() es.LocaleId = "fr-fr" return es.LocaleId end)
T("tr Parent write", function() es.Parent = workspace return es.Parent end)
T("tr Parent nil", function() es.Parent = nil return es.Parent end)
T("tr Clone", function() return es:Clone() end)
T("tr Destroy", function() local t = LT:GetTranslator("it-it") t:Destroy() return t.LocaleId, t:FormatByKey("Greeting") end)
T("tr Instance.new", function() return Instance.new("Translator") end)
T("tr IsA", function() return es:IsA("Translator"), es:IsA("Instance") end)

local cases = {
	{ "Greeting" }, { "Named", { name = "Ann", count = 5 } }, { "Named", { name = "Ann" } }, { "Named" }, { "Named", {} },
	{ "Pos", { "x", "y" } }, { "Pos", { "x" } }, { "Pos" }, { "Missing" }, { "" }, { "NoKey" }, { "Alpha" }, { "Empty" }, { "NoSrc" },
	{ "Pos", { 1, 2, 3 } }, { "Pos", "str" }, { "Pos", 5 }, { "Pos", true }, { "Pos", workspace }, { "Mixed", { "a", "b", name = "n" } }, { "Mixed", { [1] = "a", [2] = "b", [4] = "d" } }, { "Pos", setmetatable({}, { __index = function(_, k) return "meta" .. k end }) }, { "Pos", table.freeze({ "f1", "f2" }) },
	{ "Pos", { "{2}", "{1}" } }, { "Named", { name = "%s %d", count = "{count}" } }, { "Named", { count = 1 } },
	{ "Greeting", { "unused" } }, { "Greeting", 5 }, { "Named", { name = "a", count = 1, extra = 2 } },
}
for _, loc in { "es-es", "en-us", "fr-fr", "de-de", "it-it" } do
	local tr = LT:GetTranslator(loc)
	for _, c in cases do
		T(loc .. " FormatByKey " .. S(c[1]) .. " " .. S(c[2]), function() return tr:FormatByKey(c[1], c[2]) end)
	end

end
T("FormatByKey nil key", function() return es:FormatByKey() end)
T("FormatByKey num key", function() return es:FormatByKey(5) end)
T("FormatByKey table key", function() return es:FormatByKey({}) end)
T("FormatByKey dot call", function() return es.FormatByKey(es, "Greeting") end)
T("FormatByKey wrong self", function() return es.FormatByKey(workspace, "Greeting") end)
T("FormatByKey no self", function() return es.FormatByKey() end)

-- Translate
local Label = Instance.new("TextLabel")
for _, loc in { "es-es", "en-us", "de-de" } do
	local tr = LT:GetTranslator(loc)
	for _, c in { { game, "Hello" }, { Label, "Hello" }, { game, "NoKey" }, { game, "A" }, { game, "Nope" }, { game, "" }, { game, "hello" }, { game, " Hello " }, { game, "E" } } do
		T(loc .. " Translate " .. c[1].ClassName .. " " .. S(c[2]), function() return tr:Translate(c[1], c[2]) end)
	end
end
T("Translate nil ctx", function() return es:Translate(nil, "Hello") end)
T("Translate nil text", function() return es:Translate(game) end)
T("Translate num text", function() return es:Translate(game, 5) end)
T("Translate string ctx", function() return es:Translate("x", "Hello") end)
T("Translate table ctx", function() return es:Translate({}, "Hello") end)

-- live changes after a translator is made
local tr = LT:GetTranslator("es-es")
LT:SetEntry("Greeting", "es-es", "Buenas")
T("live SetEntry", function() return tr:FormatByKey("Greeting") end)
LT:SetEntries({ { Key = "New", Source = "N", Values = { ["es-es"] = "Nuevo" } } })
T("live SetEntries", function() return tr:FormatByKey("New"), { pcall(tr.FormatByKey, tr, "Greeting") } end)
LT.SourceLocaleId = "es-es"
T("after SourceLocaleId", function() return LT:GetTranslator("fr-fr"):FormatByKey("New") end)
LT.SourceLocaleId = "en-us"

-- hierarchy: a table's translator and the tables under and over it
local Root = Instance.new("LocalizationTable")
Root.Name = "Root"
Root:SetEntries({ { Key = "R", Source = "RS", Values = { ["es-es"] = "RV" } }, { Key = "Shared", Source = "SS", Values = { ["es-es"] = "Root" } }, { Source = "Free", Values = { ["es-es"] = "RootFree" } } })
local Child = Instance.new("LocalizationTable")
Child.Name = "Child"
Child:SetEntries({ { Key = "C", Source = "CS", Values = { ["es-es"] = "CV" } }, { Key = "Shared", Source = "SS", Values = { ["es-es"] = "Child" } }, { Source = "Free", Values = { ["es-es"] = "ChildFree" } } })
Child.Parent = Root
local Grand = Instance.new("Folder")
Grand.Parent = Child
local Deep = Instance.new("LocalizationTable")
Deep:SetEntries({ { Key = "D", Source = "DS", Values = { ["es-es"] = "DV" } } })
Deep.Parent = Grand
task.wait()
local trR, trC = Root:GetTranslator("es-es"), Child:GetTranslator("es-es")
local trD = Deep:GetTranslator("es-es")
T("deep sees", function() return { pcall(trD.FormatByKey, trD, "D") }, { pcall(trD.FormatByKey, trD, "C") }, { pcall(trD.FormatByKey, trD, "R") } end)
for _, k in { "R", "C", "Shared", "D" } do
	T("root " .. k, function() return trR:FormatByKey(k) end)
	T("child " .. k, function() return trC:FormatByKey(k) end)
end
T("root Translate Free", function() return trR:Translate(game, "Free"), trC:Translate(game, "Free") end)
T("Root prop", function() Child.Root = Root return Child.Root end)
T("child after Root", function() return { pcall(trC.FormatByKey, trC, "R") } end)
T("Root prop other", function() Root.Root = workspace return Root.Root end)

-- the source locale's own translator and the source text fallback
local Src = Instance.new("LocalizationTable")
Src:SetEntries({ { Key = "OnlySource", Source = "Src text", Values = {} }, { Key = "WithUs", Source = "Src2", Values = { ["en-us"] = "Us text" } }, { Key = "NoSource", Values = { ["fr-fr"] = "Fr" } } })
for _, loc in { "en-us", "es-es", "fr-fr", "en-gb", "EN-US" } do
	local tr = Src:GetTranslator(loc)
	for _, k in { "OnlySource", "WithUs", "NoSource" } do
		T("src " .. loc .. " " .. k, function() return tr:FormatByKey(k) end)
	end
	T("src " .. loc .. " Translate", function() return tr:Translate(game, "Src text"), tr:Translate(game, "Src2") end)
end
Src.SourceLocaleId = "fr-fr"
for _, loc in { "en-us", "fr-fr", "es-es" } do
	local tr = Src:GetTranslator(loc)
	T("src fr " .. loc, function() return { pcall(tr.FormatByKey, tr, "OnlySource") }, { pcall(tr.FormatByKey, tr, "NoSource") }, { pcall(tr.FormatByKey, tr, "WithUs") } end)
end

-- Translate's contexts on a table translator
local Ctx = Instance.new("LocalizationTable")
Ctx:SetEntries({
	{ Source = "Bye", Context = "Gui.Label", Values = { ["es-es"] = "gui label" } },
	{ Source = "Bye", Context = "X.Label", Values = { ["es-es"] = "x label" } },
	{ Source = "Bye", Context = "Label", Values = { ["es-es"] = "label" } },
	{ Source = "Bye", Context = "Workspace", Values = { ["es-es"] = "workspace" } },
	{ Source = "Bye", Context = "", Values = { ["es-es"] = "plain" } },
	{ Source = "Bye", Context = "aaa", Values = { ["es-es"] = "aaa" } },
	{ Source = "Yo", Context = "Label", Values = { ["es-es"] = "yo label" } },
	{ Source = "Yo", Context = "", Values = { ["es-es"] = "yo plain" } },
	{ Source = "Yo", Context = "zzz", Values = { ["es-es"] = "yo zzz" } },
	{ Source = "Ka", Context = "Gui.Label", Values = { ["fr-fr"] = "ka fr" } },
	{ Source = "Ka", Context = "", Values = { ["es-es"] = "ka plain" } },
})
local tc = Ctx:GetTranslator("es-es")
local Gui = Instance.new("ScreenGui")
Gui.Name = "Gui"
local L1 = Instance.new("TextLabel")
L1.Name = "Label"
L1.Parent = Gui
Gui.Parent = workspace
local L2 = Instance.new("TextLabel")
L2.Name = "Label"
local Inner = Instance.new("Frame")
Inner.Name = "X"
Inner.Parent = Gui
L2.Parent = Inner
local L3 = Instance.new("TextLabel")
L3.Name = "Gui.Label"
L3.Parent = workspace
for _, c in { game, workspace, Gui, L1, L2, L3, Inner } do
	T("ctx " .. (if c == game then "game" else c:GetFullName()), function() return tc:Translate(c, "Bye"), tc:Translate(c, "Yo"), tc:Translate(c, "Ka") end)
end

local Players = game:GetService("Players")
local function Try(tr, k)
	local ok, r = pcall(tr.FormatByKey, tr, k)
	return (if ok then "=" else "!") .. tostring(r)
end
local function Tr(tr, ctx, text)
	local ok, r = pcall(tr.Translate, tr, ctx, text)
	return (if ok then "=" else "!") .. tostring(r)
end
local function Make(Name, Entries, Parent)
	local L = Instance.new("LocalizationTable")
	L.Name = Name
	L:SetEntries(Entries)
	L.Parent = Parent
	return L
end
local B = Make("B", { { Key = "Shared", Source = "SharedB", Values = { ["es-es"] = "B" } }, { Key = "BOnly", Source = "BOnlySrc", Values = { ["es-es"] = "BOnlyV" } } }, LS)
local A = Make("A", { { Key = "Shared", Source = "SharedA", Values = { ["es-es"] = "A" } }, { Key = "SrcOnly", Source = "Only source", Values = {} }, { Key = "UsOnly", Source = "UsSrc", Values = { ["en-us"] = "UsVal" } } }, LS)
task.wait()
local es = LS:GetTranslatorForLocaleAsync("es-es")
local en = LS:GetTranslatorForLocaleAsync("en-us")
local fr = LS:GetTranslatorForLocaleAsync("fr-fr")
T("B then A", function() return Try(es, "Shared"), Try(es, "BOnly"), Try(es, "SrcOnly"), Try(en, "SrcOnly"), Try(en, "UsOnly"), Try(fr, "SrcOnly"), Try(en, "Shared") end)
B.Parent = nil
B.Parent = LS
task.wait()
T("A then B", function() return Try(es, "Shared"), Try(es, "BOnly") end)
A.Name = "Z"
task.wait()
T("renamed", function() return Try(es, "Shared") end)
-- nesting
local Folder = Instance.new("Folder")
Folder.Parent = LS
local N1 = Make("N1", { { Key = "Nested", Source = "NestedSrc", Values = { ["es-es"] = "NestedV" } } }, Folder)
local N2 = Make("N2", { { Key = "InTable", Source = "InTableSrc", Values = { ["es-es"] = "InTableV" } } }, A)
for _ = 1, 3 do task.wait() end
T("nested", function() return Try(es, "Nested"), Try(es, "InTable") end)
local es2 = LS:GetTranslatorForLocaleAsync("es-es")
T("nested new translator", function() return Try(es2, "Nested"), Try(es2, "InTable") end)
-- Translate: contexts
local Gui = Instance.new("ScreenGui")
Gui.Name = "Gui"
local Label = Instance.new("TextLabel")
Label.Name = "Label"
Label.Parent = Gui
Gui.Parent = workspace
local Other = Instance.new("TextLabel")
Other.Name = "Other"
Other.Parent = Gui
local C = Make("C", {
	{ Source = "Hello", Values = { ["es-es"] = "Hola" } },
	{ Source = "Hello", Context = "Workspace.Gui.Label", Values = { ["es-es"] = "Hola label" } },
	{ Source = "Bye", Context = "Gui.Label", Values = { ["es-es"] = "Adios gui label" } },
	{ Source = "Bye", Context = "Label", Values = { ["es-es"] = "Adios label" } },
	{ Source = "Bye", Context = "Gui", Values = { ["es-es"] = "Adios gui" } },
	{ Source = "Keyed", Key = "KeyedKey", Values = { ["es-es"] = "Con clave" } },
	{ Source = "NoEs", Values = { ["fr-fr"] = "Pas es" } },
	{ Source = "Hello {name}", Values = { ["es-es"] = "Hola {name}" } },
}, LS)
task.wait()
local es3 = LS:GetTranslatorForLocaleAsync("es-es")
for _, ctx in { game, workspace, Gui, Label, Other, LS } do
	T("Translate " .. (if ctx == game then "game" else ctx:GetFullName()), function() return Tr(es3, ctx, "Hello"), Tr(es3, ctx, "Bye"), Tr(es3, ctx, "Keyed"), Tr(es3, ctx, "NoEs"), Tr(es3, ctx, "Hello {name}"), Tr(es3, ctx, "Missing") end)
end
local Detached = Instance.new("TextLabel")
Detached.Name = "Label"
T("Translate detached", function() return Tr(es3, Detached, "Hello"), Tr(es3, Detached, "Bye") end)
T("Translate en", function() return Tr(LS:GetTranslatorForLocaleAsync("en-us"), game, "Hello") end)
T("FormatByKey keyless", function() return Try(es3, ""), Try(es3, "Hello") end)
T("GetTableEntries", function()
	local Out = {}
	for I, List in LS:GetTableEntries(nil) do
		table.insert(Out, I .. ":" .. #List)
	end
	return table.concat(Out, " ")
end)
T("GetTableEntries arg", function() return #LS:GetTableEntries(A), #LS:GetTableEntries(Folder) end)
T("GetTableEntries number", function() return LS:GetTableEntries(5) end)

-- the player's translator
local Player = Players.LocalPlayer
if Player then
	T("Player.LocaleId", function() return Player.LocaleId end)
	local pt = LS:GetTranslatorForPlayer(Player)
	T("player translator", function() return pt.LocaleId, Try(pt, "Shared"), Tr(pt, game, "Hello") end)
	T("player async", function() return LS:GetTranslatorForPlayerAsync(Player) end)
end
T("RobloxLocaleId write", function() LS.RobloxLocaleId = "es-es" return LS.RobloxLocaleId end)
T("SystemLocaleId write", function() LS.SystemLocaleId = "es-es" return LS.SystemLocaleId end)
T("LocaleId write", function() es.LocaleId = "fr-fr" return es.LocaleId end)
T("translator members", function() return es.Name, es.ClassName, es.Archivable, es.Parent, #es:GetChildren() end)
T("RobloxOnlyTranslate", function() return es:RobloxOnlyTranslate(game, "Hello") end)
T("GetIsLoadingInternalTranslations", function() return LS:GetIsLoadingInternalTranslations() end)

-- (the tables this probe put in the service go: the next probe's translators start from none)
for _, Child in LS:GetChildren() do
	Child:Destroy()
end

local Players = game:GetService("Players")
local function Try(tr, k)
	local ok, r = pcall(tr.FormatByKey, tr, k)
	return (if ok then "=" else "!") .. tostring(r)
end
local function Make(Name, Count, Parent, Extra)
	local L = Instance.new("LocalizationTable")
	L.Name = Name
	local E = {}
	for I = 1, Count do
		table.insert(E, { Key = Name .. I, Source = Name .. "src" .. I, Values = { ["es-es"] = Name .. "v" .. I } })
	end
	for _, X in Extra or {} do
		table.insert(E, X)
	end
	L:SetEntries(E)
	L.Parent = Parent
	return L
end
local function Tables()
	local Out = {}
	for I, List in LS:GetTableEntries(nil) do
		table.insert(Out, I .. ":" .. #List)
	end
	return table.concat(Out, " ")
end
T("tables empty", Tables)
local A = Make("A", 1, LS, { { Key = "Shared", Source = "SA", Values = { ["es-es"] = "fromA" } } })
local A1 = Make("A1", 2, A, { { Key = "Shared", Source = "SA1", Values = { ["es-es"] = "fromA1" } }, { Key = "Deep", Source = "SA1d", Values = { ["es-es"] = "deepA1" } } })
local A2 = Make("A2", 3, A1, { { Key = "Deep", Source = "SA2d", Values = { ["es-es"] = "deepA2" } } })
local F = Instance.new("Folder")
F.Name = "F"
F.Parent = LS
local F1 = Make("F1", 4, F)
local B = Make("B", 5, LS, { { Key = "Shared", Source = "SB", Values = { ["es-es"] = "fromB" } } })
local M = Instance.new("Model")
M.Parent = A
local M1 = Make("M1", 6, M)
task.wait()
T("tables", Tables)
local es = LS:GetTranslatorForLocaleAsync("es-es")
T("lookups", function() return Try(es, "Shared"), Try(es, "Deep"), Try(es, "A11"), Try(es, "A21"), Try(es, "F11"), Try(es, "M11"), Try(es, "B1") end)
local at = A:GetTranslator("es-es")
T("table A translator", function() return Try(at, "Shared"), Try(at, "Deep"), Try(at, "A21"), Try(at, "B1"), Try(at, "M11") end)
local a1t = A1:GetTranslator("es-es")
T("table A1 translator", function() return Try(a1t, "Shared"), Try(a1t, "A1"), Try(a1t, "A21") end)
T("Translate sources", function() return es:Translate(game, "SA"), es:Translate(game, "SA1"), es:Translate(game, "F1src1"), es:Translate(game, "M1src1") end)
-- tables elsewhere in the DataModel
local RS = game:GetService("ReplicatedStorage")
local R = Make("R", 7, RS)
local PG = if Players.LocalPlayer then Players.LocalPlayer:FindFirstChildOfClass("PlayerGui") else nil
local P = if PG then Make("P", 8, PG) else nil
task.wait()
T("elsewhere", function() return Tables(), Try(es, "R1"), if P then Try(es, "P1") else "-" end)
-- Root property
local Label = Instance.new("TextLabel")
Label.Parent = workspace
T("RootLocalizationTable", function() Label.RootLocalizationTable = R return es:Translate(Label, "Rsrc1"), R:GetTranslator("es-es"):Translate(Label, "Rsrc1") end)
-- locales and casts
for _, Loc in { "ab", "a", "abc", "EN-US", "es_ES", " es-es", "es-es ", "es--es", "-", "--", "12" } do
	T("ForLocaleAsync " .. S(Loc), function() local tr = LS:GetTranslatorForLocaleAsync(Loc) return tr.LocaleId, Try(tr, "A1") end)
end
T("ForLocaleAsync table", function() return LS:GetTranslatorForLocaleAsync({}).LocaleId end)
T("GetTranslator 'ab'", function() return A:GetTranslator("a").LocaleId, A:GetTranslator("").LocaleId end)
T("ForPlayer number", function() return LS:GetTranslatorForPlayer(5) end)
T("ForPlayer table", function() return LS:GetTranslatorForPlayer({}) end)
T("ForPlayerAsync workspace", function() return LS:GetTranslatorForPlayerAsync(workspace) end)
T("ForPlayerAsync number", function() return LS:GetTranslatorForPlayerAsync(5) end)
T("GetTableEntries number", function() return LS:GetTableEntries(5) end)
T("GetTableEntries none", function() return #LS:GetTableEntries() end)
-- (a client offline has no country region for its player: ree answers "US" for one, as before)
T("GetCountryRegionForPlayerAsync workspace", function() return LS:GetCountryRegionForPlayerAsync(workspace) end)
T("GetCountryRegionForPlayerAsync number", function() return LS:GetCountryRegionForPlayerAsync(5) end)

-- a player translator, and the source locale
if Players.LocalPlayer then
	local pt = LS:GetTranslatorForPlayer(Players.LocalPlayer)
	T("player", function() return pt.LocaleId, Try(pt, "A1"), pt:Translate(game, "Asrc1") end)
end
local en = LS:GetTranslatorForLocaleAsync("en-us")
local Us = Make("Us", 1, LS, { { Key = "UsKey", Source = "UsSource", Values = { ["en-us"] = "UsValue" } }, { Key = "Bare", Source = "BareSource", Values = {} } })
Us.SourceLocaleId = "fr-fr"
task.wait()
local frt = LS:GetTranslatorForLocaleAsync("fr-fr")
T("source locales", function() return Try(en, "UsKey"), Try(en, "Bare"), Try(frt, "Bare"), Try(en, "Us1"), Try(frt, "Us1") end)
T("translate source locale", function() return en:Translate(game, "BareSource"), frt:Translate(game, "BareSource"), en:Translate(game, "UsSource") end)

-- (the tables this probe put in the service go: the next probe's translators start from none)
for _, Child in LS:GetChildren() do
	Child:Destroy()
end

-- which value
local L = Instance.new("LocalizationTable")
L.SourceLocaleId = "en-us"
L:SetEntries({
	{ Key = "es", Source = "S-es", Values = { ["es-es"] = "v-es-es" } },
	{ Key = "lang", Source = "S-lang", Values = { ["es"] = "v-es" } },
	{ Key = "both", Source = "S-both", Values = { ["es"] = "v-es", ["es-mx"] = "v-es-mx" } },
	{ Key = "two", Source = "S-two", Values = { ["es-mx"] = "v-es-mx", ["es-ar"] = "v-es-ar" } },
	{ Key = "pt", Source = "S-pt", Values = { ["pt-br"] = "v-pt-br" } },
	{ Key = "zh", Source = "S-zh", Values = { ["zh-cn"] = "v-zh-cn", ["zh-tw"] = "v-zh-tw" } },
	{ Key = "src", Source = "S-src", Values = {} },
	{ Key = "srcempty", Source = "", Values = { ["fr-fr"] = "x" } },
	{ Key = "emptyval", Source = "S-ev", Values = { ["es-es"] = "" } },
})
for _, Loc in { "es-es", "es-mx", "es", "es-ar", "es-es ", " es-es", "es--es", "es-es-x", "esp", "es_mx", "pt-pt", "pt", "zh-hk", "zh", "zh-cn", "en-us", "en-gb", "en", "eng", "e", "en-", "-us", "fr-fr" } do
	local Tr = L:GetTranslator(Loc)
	local Parts = {}
	for _, K in { "es", "lang", "both", "two", "pt", "zh", "src", "srcempty", "emptyval" } do
		table.insert(Parts, K .. Try(Tr, K))
	end
	print(S(Loc), table.concat(Parts, " "))
	print("  translate", Tr:Translate(game, "S-es"), Tr:Translate(game, "S-lang"), Tr:Translate(game, "S-both"), Tr:Translate(game, "S-src"))
end
for _, Src in { "en", "en-gb", "EN_US", "es-mx", "", "xx" } do
	local M = Instance.new("LocalizationTable")
	M.SourceLocaleId = Src
	M:SetEntries({ { Key = "k", Source = "Src", Values = {} } })
	local Parts = {}
	for _, Loc in { "en-us", "en", "en-gb", "es-es", "es", "", "xx" } do
		table.insert(Parts, Loc .. Try(M:GetTranslator(Loc), "k"))
	end
	print("source " .. S(Src), table.concat(Parts, " "))
end

-- the service translator when the first table with the key has no text for the locale
local function Make(Name, Entries, Parent)
	local L = Instance.new("LocalizationTable")
	L.Name = Name
	L:SetEntries(Entries)
	L.Parent = Parent
	return L
end
local P = Make("P", { { Key = "K", Source = "PS", Values = { ["fr-fr"] = "Pfr" } }, { Key = "K2", Source = "Src2", Values = {} } }, LS)
local Q = Make("Q", { { Key = "K", Source = "QS", Values = { ["es-es"] = "Qes" } }, { Key = "K2", Source = "Src2", Values = { ["es-es"] = "Q2" } }, { Source = "PS", Values = { ["es-es"] = "Qfree" } } }, LS)
Q.SourceLocaleId = "es-es"
task.wait()
local es = LS:GetTranslatorForLocaleAsync("es-es")
local en = LS:GetTranslatorForLocaleAsync("en-us")
T("first lacks locale", function() return { pcall(es.FormatByKey, es, "K") }, { pcall(en.FormatByKey, en, "K") }, { pcall(es.FormatByKey, es, "K2") } end)
T("translate first lacks", function() return es:Translate(game, "PS"), es:Translate(game, "Src2"), en:Translate(game, "QS") end)
-- a table translator's own source-locale rules
local tq = Q:GetTranslator("es-es")
local tp = P:GetTranslator("es-es")
T("table translator lacks", function() return { pcall(tp.FormatByKey, tp, "K") }, { pcall(tq.FormatByKey, tq, "K") }, { pcall(tp.FormatByKey, tp, "K2") } end)

-- Changed on entry writes
local W = Instance.new("LocalizationTable")
local Seen = {}
W.Changed:Connect(function(Name) table.insert(Seen, Name) end)
W:SetEntries({ V("k", "", "") })
W:SetEntry("k", "es-es", "v")
W:SetContents("[]")
W.SourceLocaleId = "fr-fr"
task.wait()
T("Changed", function() return table.concat(Seen, ",") end)
-- Roblox-only members
local Tr = W:GetTranslator("es-es")
for _, Call in {
	{ "RobloxOnlyTranslate", function() return Tr:RobloxOnlyTranslate(game, "x") end },
	{ "SetIsExemptFromUGCAnalytics", function() return W:SetIsExemptFromUGCAnalytics(true) end },
	{ "GetIsLoadingInternalTranslations", function() return LS:GetIsLoadingInternalTranslations() end },
	{ "IsLoadingInternalTranslationsSettingChanged", function() return LS:IsLoadingInternalTranslationsSettingChanged(true) end },
	{ "SetRobloxLocaleId", function() return LS:SetRobloxLocaleId("es-es") end },
	{ "StartTextScraper", function() return LS:StartTextScraper() end },
	{ "StopTextScraper", function() return LS:StopTextScraper() end },
	{ "PromptDownloadGameTableToCSV", function() return LS:PromptDownloadGameTableToCSV(W) end },
	{ "PromptExportToCSVs", function() return LS:PromptExportToCSVs() end },
	{ "PromptImportFromCSVs", function() return LS:PromptImportFromCSVs() end },
	{ "PromptUploadCSVToGameTable", function() return LS:PromptUploadCSVToGameTable() end },
	{ "LocaleManifest", function() return LS.LocaleManifest end },
	{ "IsExemptFromUGCAnalytics", function() return W.IsExemptFromUGCAnalytics end },
	{ "AutoTranslateWillRun", function() return LS.AutoTranslateWillRun end },
	{ "GetCorescriptLocalizations", function() return #LS:GetCorescriptLocalizations() end },
} do
	T(Call[1], Call[2])
end

-- (the tables this probe put in the service go: the next probe's translators start from none)
for _, Child in LS:GetChildren() do
	Child:Destroy()
end

-- contexts
local C = Instance.new("LocalizationTable")
C:SetEntries({
	{ Source = "P", Context = "A.Label", Values = { ["es-es"] = "partial" } },
	{ Source = "P", Context = "zz", Values = { ["es-es"] = "zz" } },
	{ Source = "Q", Context = "ui.Label", Values = { ["es-es"] = "string suffix" } },
	{ Source = "Q", Context = "zz", Values = { ["es-es"] = "zz" } },
	{ Source = "R", Context = "Workspace.Gui.Label", Values = { ["es-es"] = "full" } },
	{ Source = "R", Context = "Game.Workspace.Gui.Label", Values = { ["es-es"] = "longer" } },
	{ Source = "R", Context = "zz", Values = { ["es-es"] = "zz" } },
	{ Source = "S", Context = "label", Values = { ["es-es"] = "lower" } },
	{ Source = "S", Context = "zz", Values = { ["es-es"] = "zz" } },
	{ Source = "U", Context = "Gui", Values = { ["es-es"] = "gui" } },
	{ Source = "U", Context = "Label", Values = { ["es-es"] = "label" } },
	{ Source = "U", Context = "Workspace", Values = { ["es-es"] = "ws" } },
	{ Key = "k1", Source = "V", Context = "Label", Values = { ["es-es"] = "keyed label" } },
	{ Source = "V", Context = "Label", Values = { ["es-es"] = "free label" } },
})
local Gui = Instance.new("ScreenGui")
Gui.Name = "Gui"
local Label = Instance.new("TextLabel")
Label.Name = "Label"
Label.Parent = Gui
Gui.Parent = workspace
local Ce = C:GetTranslator("es-es")
for _, S in { "P", "Q", "R", "S", "U", "V" } do
	print("ctx " .. S, Ce:Translate(Label, S), Ce:Translate(Gui, S), Ce:Translate(workspace, S), Ce:Translate(game, S))
end

-- more locales
local V = Instance.new("LocalizationTable")
V:SetEntries({
	{ Key = "ar_us", Source = "s1", Values = { ["es-ar"] = "es-ar", ["es-us"] = "es-us" } },
	{ Key = "419", Source = "s2", Values = { ["es-419"] = "es-419", ["es-ar"] = "es-ar" } },
	{ Key = "zh", Source = "s3", Values = { ["zh-cn"] = "zh-cn", ["zh-tw"] = "zh-tw", ["zh-hk"] = "zh-hk" } },
	{ Key = "zhcn", Source = "s4", Values = { ["zh-cn"] = "zh-cn" } },
	{ Key = "pt", Source = "s5", Values = { ["pt-pt"] = "pt-pt", ["pt-br"] = "pt-br" } },
	{ Key = "gb", Source = "s6", Values = { ["en-gb"] = "en-gb" } },
	{ Key = "mx", Source = "s7", Values = { ["es-mx"] = "es-mx", ["es-es"] = "es-es" } },
	{ Key = "fr", Source = "s8", Values = { ["fr-ca"] = "fr-ca", ["fr-be"] = "fr-be" } },
	{ Key = "de", Source = "s9", Values = { ["de-at"] = "de-at", ["de-ch"] = "de-ch" } },
})
V.SourceLocaleId = "ja-jp"
for _, Loc in { "es-es", "es-mx", "es", "es-419", "es-co", "zh-cn", "zh-tw", "zh-hk", "zh-sg", "zh-mo", "zh", "pt-br", "pt-pt", "pt", "pt-ao", "en-us", "en", "fr-fr", "fr", "de-de", "de" } do
	local Tr = V:GetTranslator(Loc)
	local Parts = {}
	for _, K in { "ar_us", "419", "zh", "zhcn", "pt", "gb", "mx", "fr", "de" } do
		table.insert(Parts, K .. Try(Tr, K))
	end
	print("val " .. Loc, table.concat(Parts, " "))
end
