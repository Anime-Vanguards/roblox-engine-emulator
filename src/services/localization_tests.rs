//! LocalizationTable, Translator and LocalizationService's translators through the script API (localization.rs). Every
//! expected line is one a 0.747 playing client printed for the same script (tools/parity/expected/localization_*.txt).

use std::rc::Rc;

use mlua::prelude::*;

use crate::{
    Engine, Options, RunContext,
    lua::{self as engine_lua, globals::CallOutcome},
    scheduler,
};

fn start(options: Options) -> (Lua, Rc<Engine>) {
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    (lua, engine)
}

/// Runs `source` as a script for a few frames: the output lines ("kind: message")
fn run_with(options: Options, source: &str) -> Vec<String> {
    let (lua, engine) = start(options);
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        crate::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(&lua, &engine, script).expect("environment");
    let function = lua.load(format!("{HELPERS}\n{source}")).set_name("=Test").set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(&lua, &engine, function, LuaMultiValue::new()).expect("call");
    if let CallOutcome::Failed(message, _) = call.outcome().unwrap() {
        panic!("test script failed: {message}");
    }
    scheduler::run_until(&lua, &engine, 0.2).unwrap();
    if let Err(message) = call.finish(&lua, &engine) {
        panic!("test script failed: {message}");
    }
    engine.state.borrow().output.entries.iter().map(|entry| format!("{}: {}", entry.kind, entry.message)).collect()
}

/// The lines a client script printed, without the "print: " before each (anything else it wrote fails the test)
fn printed_with(options: Options, source: &str) -> Vec<String> {
    let output = run_with(options, source);
    let others: Vec<&String> = output.iter().filter(|line| !line.starts_with("print: ")).collect();
    assert!(others.is_empty(), "{others:?}");
    output.iter().map(|line| line["print: ".len()..].to_string()).collect()
}

fn printed(source: &str) -> Vec<String> {
    printed_with(Options::default(), source)
}

fn server() -> Options {
    Options { run_context: RunContext::Server, ..Options::default() }
}

const HELPERS: &str = r#"
local function Try(F, ...)
    local Ok, Result = pcall(F, ...)
    return (if Ok then "=" else "!") .. tostring(Result)
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
local LS = game:GetService("LocalizationService")
"#;

/// A UI library's translator: a table of its own, a translator for the Roblox locale
/// and one for en-us, FormatByKey with arguments in a pcall and LocaleId read when it fails. GetTranslator gave nil before.
#[test]
fn a_ui_library_translates_through_a_table_of_its_own() {
    let lines = printed(
        r#"
        local Table = Instance.new("LocalizationTable")
        Table.Name = "LocalizationTable"
        Table.Archivable = false
        Table.SourceLocaleId = "en-us"
        Table:SetEntries({
            { Key = "Library.Action.Cancel", Source = "Cancel", Values = { ["es-es"] = "Cancelar" } },
            { Key = "Library.Label.Count", Source = "{count} items", Values = { ["es-es"] = "{count} elementos" } },
        })
        local Translator = Table:GetTranslator(LS.RobloxLocaleId)
        print(Translator.ClassName, Translator.Name, Translator.LocaleId)
        print(Translator:FormatByKey("Library.Action.Cancel"), Translator:FormatByKey("Library.Label.Count", { count = 3 }))
        local Ok, Problem = pcall(function() return Translator:FormatByKey("Missing.Key") end)
        print(Ok, Problem, Translator.LocaleId == "zh-cjv")
        local Spanish = Table:GetTranslator("es-es")
        print(Spanish:FormatByKey("Library.Action.Cancel"), Spanish:FormatByKey("Library.Label.Count", { count = "7" }))
        "#,
    );
    assert_eq!(
        lines,
        [
            "Translator Instance en-us",
            "Cancel 3.00 items",
            "false Key Missing.Key not found for locale en-us false",
            "Cancelar 7 elementos",
        ]
    );
}

/// SetEntries' checks and the order GetEntries lists entries in: by Source group in the order the groups were made, each by
/// Context then Key; a refused call changes nothing
#[test]
fn set_entries_checks_its_entries_and_lists_them_by_source_group() {
    let lines = printed(
        r#"
        local L = Instance.new("LocalizationTable")
        for _, Case in {
            { "number", 5 }, { "dict", { x = V("a", "", "") } }, { "item", { 5 } }, { "empty item", { {} } },
            { "key", { { Key = 5, Source = "x", Values = {} } } }, { "no key", { { Source = "", Values = {} } } },
            { "example", { { Key = "a", Example = true } } }, { "values", { { Key = "a" } } }, { "values array", { { Key = "a", Values = { "x" } } } },
            { "value", { { Key = "a", Values = { ["es-es"] = 5 } } } }, { "dup", { V("a", "x", ""), V("a", "y", "") } }, { "token", { V("a", "a{b", "") } },
        } do
            L:SetEntries({ V("keep", "", "") })
            print(Case[1], Try(L.SetEntries, L, Case[2]), Keys(L))
        end
        print(Try(L.SetEntries, L))
        L:SetEntries({ V("b", "", ""), V("a", "", ""), V("k1", "zz", ""), V("", "free", "ctx"), V("", "free", ""), V("k2", "aa", ""), V("c", "", "a"), { Source = "free", Values = { ["es-es"] = "dropped" } } })
        print(Keys(L))
        L:SetEntries({ { Key = "k", Values = { ["ES-ES"] = "x", ["es-es"] = "y", ["Fr_fr"] = "z", [""] = "e", [5] = "n" } } })
        local Values = L:GetEntries()[1].Values
        print(Values["es-es"], Values["fr-fr"], Values[""], Values["5"])
        "#,
    );
    assert_eq!(
        lines,
        [
            "number !entries was not an array. keep||",
            "dict !entries was not an array. keep||",
            "item !Entry at index 1 was not a table. keep||",
            "empty item !Entry at index 1 was not a table. keep||",
            "key !Entry at index 1 had invalid Key/Source/Context: Entry at index 1 \"Key\" was not a string. keep||",
            "no key !Entry at index 1 had invalid Key/Source/Context: Cannot create entry with empty source text and no key. keep||",
            "example !Entry at index 1 \"Example\" was not a string. keep||",
            "values !Entry at index 1 had was missing \"Values\". keep||",
            "values array !Entry at index 1 [\"Values\"] was not a table. keep||",
            "value !Entry at index 1 \"es-es\" was not a string. keep||",
            "dup !Entry at index 2 has the same (key) or (key,source,context) tuple as another entry. key: \"a\", source: \"y\", context: \"\" keep||",
            "token !Entry at index 1 had invalid Key/Source/Context: \"a{b\". Incomplete parameter token at end of string. keep||",
            "!Argument 1 missing or nil",
            "a|| b|| c||a k1|zz| |free| |free|ctx k2|aa|",
            "x z nil n",
        ]
    );
}

/// The entry methods: SetEntry and SetEntryValue make what they name, a key or a Source and Context; a change that would clash
/// raises, one that leaves no key and no Source loses the entry; a group stays where it was when its entries go
#[test]
fn entry_methods_find_make_and_refile_entries() {
    let lines = printed(
        r#"
        local L = Instance.new("LocalizationTable")
        L:SetEntry("K1", "ES_es", "Uno")
        L:SetEntryValue("K2", "Two", "", "es-es", "Dos")
        L:SetEntryValue("K2", "Other", "", "fr-fr", "Deux")
        L:SetEntryValue("", "Free", "c", "es-es", "Libre")
        print(Keys(L), L:GetString("es-es", "K1"), L:GetString("FR_FR", "K2"))
        print(Try(L.GetString, L, "de-de", "K1"), Try(L.GetString, L, "es-es", "Nope"), Try(L.SetEntry, L, "", "es-es", "x"))
        print(Try(L.SetEntryKey, L, "K2", "", "", "K1"), Try(L.SetEntryValue, L, "K3", "s", "", "es-es", "{ }"), Keys(L))
        L:SetEntryContext("K1", "", "", "zz")
        L:SetEntrySource("K2", "Two", "", "S9")
        L:SetEntryValue("K4", "Two", "", "es-es", "Cuatro")
        print(Keys(L))
        print(Try(L.SetEntryKey, L, "K1", "", "", ""), Keys(L))
        L:RemoveKey("K2")
        L:RemoveEntry("", "Free", "c")
        L:SetEntryValue("K5", "Free", "", "es-es", "v")
        L:RemoveTargetLocale("ES-ES")
        L:RemoveEntryValue("K5", "", "", "ES-ES")
        print(Keys(L), #L:GetEntries()[2].Values == nil, next(L:GetEntries()[2].Values))
        L:SetEntryExample("missing", "", "", "ex")
        print(Keys(L))
        "#,
    );
    assert_eq!(
        lines,
        [
            "K1|| K2|Two| |Free|c Uno Deux",
            "!LocalizationTable entry with key \"K1\" does not contain locale \"de-de\" !LocalizationTable does not contain key \"Nope\" !Cannot create entry with empty source text and no key.",
            "!The modified entry would conflict with an existing entry. !Invalid parameter name \" \". Parameter names can only contain letters (A-Z, a-z), digits (0-9), and underscores (_). K1|| K2|Two| |Free|c",
            "K1||zz K2|S9| |Free|c K4|Two|",
            "!Failed to add entry to index after modifying. K2|S9| |Free|c K4|Two|",
            "K5|Free| K4|Two| false es-es Cuatro",
            "K5|Free| K4|Two|",
        ]
    );
}

/// The template parser's errors, as SetEntries reports them (the template written before the message, or not)
#[test]
fn sources_are_parsed_as_templates() {
    let lines = printed(
        r#"
        for _, Source in { "{}", "{{x}}", "{ }", "{0}", "{1.5}", "{1name}", "{", "a{b", "}", "a{}b", "{1:}", "{:int}", "{1:bogus}", "{1} {name}", "{name} {1}" } do
            local L = Instance.new("LocalizationTable")
            print(Source, Try(L.SetEntries, L, { { Key = "t", Source = Source, Values = {} } }), Keys(L))
        end
        "#,
    );
    let quoted = |source: &str, message: &str| format!("{source} !Entry at index 1 had invalid Key/Source/Context: \"{source}\". {message} ");
    assert_eq!(
        lines,
        [
            "{} =nil t||".to_string(),
            "{{x}} =nil t|{{x}}|".to_string(),
            quoted("{ }", "Invalid parameter name \" \". Parameter names can only contain letters (A-Z, a-z), digits (0-9), and underscores (_)."),
            quoted("{0}", "Parameter index \"0\" out of range. Parameter indexes must be > 0."),
            quoted("{1.5}", "Invalid parameter index \"1.5\"."),
            quoted("{1name}", "Invalid parameter index \"1name\"."),
            "{ !Entry at index 1 had invalid Key/Source/Context: Incomplete parameter or unescaped '{' at end of string. ".to_string(),
            quoted("a{b", "Incomplete parameter token at end of string."),
            "} !Entry at index 1 had invalid Key/Source/Context: Unescaped '}' at end of string, not part of a parameter token. ".to_string(),
            quoted("a{}b", "Empty parameter token at [2]"),
            quoted("{1:}", "Empty parameter format specifier at [3]"),
            quoted("{:int}", "Empty parameter name at [1]"),
            quoted("{1:bogus}", "Unknown format specifier \"bogus\""),
            quoted("{1} {name}", "Inconsistent parameter types. Found named parameter \"name\" after numeric parameter."),
            quoted("{name} {1}", "Inconsistent parameter types. Found numeric parameter \"1\" after named parameter."),
        ]
    );
}

/// FormatByKey's arguments and formats
#[test]
fn format_by_key_writes_its_arguments() {
    let lines = printed(
        r#"
        local L = Instance.new("LocalizationTable")
        L:SetEntries({
            { Key = "p", Source = "{2} and {1}", Values = {} }, { Key = "n", Source = "{name}!", Values = {} },
            { Key = "f", Source = "{1}|{1:int}|{1:fixed}|{1:num}|{1:hex}|{1:HEX}|{1:translate}", Values = {} },
            { Key = "d", Source = "{1:datetime} {1:iso8601}", Values = {} }, { Key = "Hello", Source = "Hello", Values = { ["es-es"] = "Hola" } },
            { Key = "t", Source = "[{1:translate}]", Values = { ["es-es"] = "<{1:translate}>" } },
        })
        local T = L:GetTranslator("en-us")
        print(T:FormatByKey("p", { "a", "b" }), Try(T.FormatByKey, T, "p"), Try(T.FormatByKey, T, "p", {}), Try(T.FormatByKey, T, "p", { "a" }))
        print(Try(T.FormatByKey, T, "p", { x = 1 }), Try(T.FormatByKey, T, "p", "str"), Try(T.FormatByKey, T, "n", { "a" }), Try(T.FormatByKey, T, "n", { nom = 1 }))
        print(T:FormatByKey("n", { name = true }), T:FormatByKey("n", { name = Vector3.new(1, 2, 3) }), T:FormatByKey("n", { name = workspace }), T:FormatByKey("n", { name = Enum.Material.Plastic }))
        for _, N in { 255.5, -3, 2.5, 1234567.891, 0.1 + 0.2, true } do
            print(T:FormatByKey("f", { N }))
        end
        print(Try(T.FormatByKey, T, "f", { "x" }), Try(T.FormatByKey, T, "f", { 1e20 }))
        print(T:FormatByKey("d", { 1700000000 }), T:FormatByKey("d", { "2023-02-30T00:00:00Z" }), T:FormatByKey("d", { 1.5 }))
        print(Try(T.FormatByKey, T, "d", { -1 }), Try(T.FormatByKey, T, "d", { 4294967296 }), Try(T.FormatByKey, T, "d", { "2023-11-14" }), Try(T.FormatByKey, T, "d", { "1969-12-31T23:59:59Z" }))
        local E = L:GetTranslator("es-es")
        print(E:FormatByKey("t", { "Hello" }), T:FormatByKey("t", { "Hello" }), Try(L:GetTranslator("fr-fr").FormatByKey, L:GetTranslator("fr-fr"), "f", { 1 }))
        local De = Instance.new("LocalizationTable")
        De.SourceLocaleId = "de-de"
        De:SetEntries({ { Key = "num", Source = "{1:num}", Values = {} } })
        local Fr = De:Clone()
        Fr.SourceLocaleId = "FR_fr"
        print(De:GetTranslator("de-at"):FormatByKey("num", { -9876.125 }), Fr:GetTranslator("fr-ca"):FormatByKey("num", { 1234.5 }) == "1\u{a0}234,50", Fr.SourceLocaleId)
        "#,
    );
    assert_eq!(
        lines,
        [
            "b and a !Found parameter token, but had no parameters !Parameter index \"2\" was out of range of empty array !Parameter index \"2\" was out of range [1-1]",
            "!Expected array table, but table contained named arguments. !Found parameter token but parameters was not a table !Parameter index \"0\" was out of range [1-1] !Parameter name \"name\" not found in table",
            "1! 1, 2, 3! Instance! Plastic!",
            "255.50|256|255.50|255.50|ff|FF|255.5",
            "-3.00|-3|-3.00|-3.00|fffffffffffffffd|FFFFFFFFFFFFFFFD|-3",
            "2.50|2|2.50|2.50|2|2|2.5",
            "1234567.89|1234568|1234567.89|1,234,567.89|12d687|12D687|1234567.8910000000615",
            "0.30|0|0.30|0.30|0|0|0.30000000000000004441",
            "1|1|1.00|1.00|1|1|true",
            "!Format type int requires numerical parameter.\n !Number out of range for hex/HEX format (100000000000000000000.000000)\n",
            "2023-11-14 22:13:20 2023-11-14T22:13:20Z 2023-02-30 00:00:00 2023-02-30T00:00:00Z 1970-01-01 00:00:02 1970-01-01T00:00:02Z",
            "!Error interpreting number as a date/time (-1) out of range.\n !Error interpreting number as a date/time (0) out of range.\n !Error interpreting string as a date/time. (2023-11-14)  Must be of the form YYYY-mm-ddTHH:MM:SSZ\n !Error interpreting string as a date/time. (1969-12-31T23:59:59Z)  Date/time fields failed bound check\n",
            "<Hola> [Hello] !Key f not found for locale fr-fr",
            "-9.876,12 true fr-fr",
        ]
    );
}

/// Which text a translator's locale finds: the locale's, its language's, the paired locale's, another of its language's, then
/// the Source of a table whose source locale has its language
#[test]
fn translators_fall_back_by_language() {
    let lines = printed(
        r#"
        local L = Instance.new("LocalizationTable")
        L.SourceLocaleId = "ja-jp"
        L:SetEntries({
            { Key = "lang", Source = "s1", Values = { ["es"] = "es", ["es-mx"] = "es-mx" } },
            { Key = "two", Source = "s2", Values = { ["es-mx"] = "es-mx", ["es-ar"] = "es-ar" } },
            { Key = "zh", Source = "s3", Values = { ["zh-cn"] = "zh-cn", ["zh-tw"] = "zh-tw" } },
            { Key = "fr", Source = "s4", Values = { ["fr-ca"] = "fr-ca", ["fr-be"] = "fr-be" } },
            { Key = "empty", Source = "s5", Values = { ["es-es"] = "" } },
            { Key = "ja", Source = "s6", Values = {} },
        })
        for _, Locale in { "es-es", "es-co", "zh-hk", "zh-mo", "zh", "fr-fr", "fr", "ja", "en-us" } do
            local T = L:GetTranslator(Locale)
            local Out = {}
            for _, Key in { "lang", "two", "zh", "fr", "empty", "ja" } do
                table.insert(Out, Key .. Try(T.FormatByKey, T, Key))
            end
            print(Locale, table.concat(Out, " "))
        end
        "#,
    );
    assert_eq!(
        lines,
        [
            "es-es lang=es two=es-mx zh!Key zh not found for locale es-es fr!Key fr not found for locale es-es empty!Key empty not found for locale es-es ja!Key ja not found for locale es-es",
            "es-co lang=es two=es-ar zh!Key zh not found for locale es-co fr!Key fr not found for locale es-co empty!Key empty not found for locale es-co ja!Key ja not found for locale es-co",
            "zh-hk lang!Key lang not found for locale zh-hk two!Key two not found for locale zh-hk zh=zh-tw fr!Key fr not found for locale zh-hk empty!Key empty not found for locale zh-hk ja!Key ja not found for locale zh-hk",
            "zh-mo lang!Key lang not found for locale zh-mo two!Key two not found for locale zh-mo zh!Key zh not found for locale zh-mo fr!Key fr not found for locale zh-mo empty!Key empty not found for locale zh-mo ja!Key ja not found for locale zh-mo",
            "zh lang!Key lang not found for locale zh two!Key two not found for locale zh zh!Key zh not found for locale zh fr!Key fr not found for locale zh empty!Key empty not found for locale zh ja!Key ja not found for locale zh",
            "fr-fr lang!Key lang not found for locale fr-fr two!Key two not found for locale fr-fr zh!Key zh not found for locale fr-fr fr=fr-ca empty!Key empty not found for locale fr-fr ja!Key ja not found for locale fr-fr",
            "fr lang!Key lang not found for locale fr two!Key two not found for locale fr zh!Key zh not found for locale fr fr=fr-be empty!Key empty not found for locale fr ja!Key ja not found for locale fr",
            "ja lang=s1 two=s2 zh=s3 fr=s4 empty=s5 ja=s6",
            "en-us lang!Key lang not found for locale en-us two!Key two not found for locale en-us zh!Key zh not found for locale en-us fr!Key fr not found for locale en-us empty!Key empty not found for locale en-us ja!Key ja not found for locale en-us",
        ]
    );
}

/// The tables each translator reads (a table's: it and the tables it is in; the service's: the tables in it that only tables
/// hold, a table's own after those in it), read as they are at each call; GetTableEntries lists the service's in that order
#[test]
fn translators_read_their_tables_live() {
    let lines = printed(
        r#"
        local function Make(Name, Entries, Parent)
            local L = Instance.new("LocalizationTable")
            L.Name = Name
            L:SetEntries(Entries)
            L.Parent = Parent
            return L
        end
        for _, Child in LS:GetChildren() do
            Child:Destroy()
        end
        local es = LS:GetTranslatorForLocaleAsync("es-es")
        print(Try(es.FormatByKey, es, "Shared"))
        local A = Make("A", { { Key = "Shared", Source = "SA", Values = { ["es-es"] = "fromA" } }, { Key = "Lacks", Source = "L", Values = {} } }, LS)
        local A1 = Make("A1", { { Key = "Shared", Source = "SA1", Values = { ["es-es"] = "fromA1" } }, V("x", "y", "") }, A)
        local Folder = Instance.new("Folder")
        Folder.Parent = LS
        Make("F1", { { Key = "Hidden", Source = "H", Values = { ["es-es"] = "hidden" } } }, Folder)
        local B = Make("B", { { Key = "Lacks", Source = "L", Values = { ["es-es"] = "fromB" } }, V("b1", "b", ""), V("b2", "b", "") }, LS)
        print(Try(es.FormatByKey, es, "Shared"), Try(es.FormatByKey, es, "Lacks"), Try(es.FormatByKey, es, "Hidden"), es:Translate(game, "SA"))
        local Lists = {}
        for _, List in LS:GetTableEntries() do
            table.insert(Lists, #List)
        end
        print(table.concat(Lists, " "))
        local A1t, At = A1:GetTranslator("es-es"), A:GetTranslator("es-es")
        print(Try(A1t.FormatByKey, A1t, "Lacks"), Try(At.FormatByKey, At, "Shared"), Try(A1t.FormatByKey, A1t, "Shared"))
        A1.Parent = nil
        print(Try(es.FormatByKey, es, "Shared"), Try(A1t.FormatByKey, A1t, "Lacks"))
        local en = LS:GetTranslatorForLocaleAsync("EN_GB")
        print(en.LocaleId, Try(en.FormatByKey, en, "Lacks"), en:Translate(game, "L"))
        "#,
    );
    assert_eq!(
        lines,
        [
            "!Key Shared not found for locale es-es",
            "=fromA1 =fromB !Key Hidden not found for locale es-es fromA",
            "2 2 3 0",
            "!Key Lacks not found for locale es-es =fromA =fromA1",
            "=fromA !Key Lacks not found for locale es-es",
            "en-gb =L L",
        ]
    );
}

/// Translate: the translation of the entry of that Source whose Context fits the context's full name best (the last among the
/// best), ASCII case aside; entries with no translation for the locale are passed over
#[test]
fn translate_picks_the_entry_whose_context_fits() {
    let lines = printed(
        r#"
        local L = Instance.new("LocalizationTable")
        L:SetEntries({
            { Source = "Bye", Context = "Gui.Label", Values = { ["es-es"] = "gui label" } },
            { Source = "Bye", Context = "label", Values = { ["es-es"] = "label" } },
            { Source = "Bye", Context = "Workspace", Values = { ["es-es"] = "workspace" } },
            { Source = "Bye", Context = "", Values = { ["es-es"] = "plain" } },
            { Source = "Bye", Context = "aaa", Values = { ["es-es"] = "aaa" } },
            { Source = "Ka", Context = "Gui.Label", Values = { ["fr-fr"] = "ka fr" } },
            { Source = "Ka", Context = "", Values = { ["es-es"] = "ka plain" } },
            { Key = "k", Source = "Bad", Values = { ["es-es"] = "{ }" } },
        })
        local T = L:GetTranslator("es-es")
        local Gui = Instance.new("ScreenGui")
        Gui.Name = "Gui"
        local Label = Instance.new("TextLabel")
        Label.Name = "Label"
        Label.Parent = Gui
        Gui.Parent = workspace
        local Other = Instance.new("TextLabel")
        Other.Name = "Label"
        for _, Context in { Label, Other, Gui, workspace } do
            print(T:Translate(Context, "Bye"), T:Translate(Context, "Ka"), T:Translate(Context, "Nope"), T:Translate(Context, "Bad"), T:Translate(Context, ""))
        end
        print(Try(T.Translate, T, nil, "Bye"), Try(T.Translate, T, "x", "Bye"), Try(T.Translate, T, game), T:Translate(game, 5))
        "#,
    );
    assert_eq!(
        lines,
        [
            "gui label ka plain Nope Bad ",
            "gui label ka plain Nope Bad ",
            "label ka plain Nope Bad ",
            "workspace ka plain Nope Bad ",
            "!Context must be an Instance. !Unable to cast value to Object !Argument 2 missing or nil 5",
        ]
    );
}

/// A table's contents: GetContents' JSON, SetContents' errors, and a copy that reads its own (losing the Examples)
#[test]
fn contents_are_the_json_a_client_writes_and_reads() {
    let lines = printed(
        r#"
        local L = Instance.new("LocalizationTable")
        L:SetEntries({ { Key = "k", Source = "s", Context = "c", Example = "e", Values = { ["fr-fr"] = "w", ["es-es"] = "v" } }, { Source = "free", Values = {} }, { Key = "q\"\n\1", Values = { ["es-es"] = "{}" } } })
        print(L:GetContents())
        local Copy = L:Clone()
        print(Copy:GetEntries()[1].Example, L:GetEntries()[1].Example, Copy:GetContents() == L:GetContents(), Try(Copy:GetTranslator("es-es").FormatByKey, Copy:GetTranslator("es-es"), "k"))
        for _, Text in { "", "not json", "[] x", "{}", "[5]", '[{"key":"k"}]', '[{"key":"k","values":[]}]', '[{"key":"k","values":{"es-es":5}}]', '[{"source":"","values":{}}]', '[{"key":"a","source":"{ }","values":{}}]' } do
            local C = Instance.new("LocalizationTable")
            C:SetEntries({ V("keep", "", "") })
            print(Try(C.SetContents, C, Text), Keys(C))
        end
        local C = Instance.new("LocalizationTable")
        C:SetContents('[{"key":"k","source":"s","example":"e1","examples":"e2","values":{"ES_ES":"v","":"e"}}]')
        print(C:GetEntries()[1].Example, C:GetEntries()[1].Values["es-es"], C:GetEntries()[1].Values[""], Try(function() return L.Contents end))
        "#,
    );
    assert_eq!(
        lines,
        [
            r#"[{"key":"k","context":"c","examples":"e","source":"s","values":{"es-es":"v","fr-fr":"w"}},{"source":"free","values":{}},{"key":"q\"\n\u0001","values":{"es-es":""}}]"#,
            " e false =v",
            "!JSON parse error: The document is empty. keep||",
            "!JSON parse error: Unspecific syntax error. keep||",
            "!JSON parse error: The document root must not be followed by other values. keep||",
            "!Root is not an array keep||",
            "!Entry is not an object keep||",
            "!Entry does not have \"values\" keep||",
            "!Entry \"values\" is not an object keep||",
            "!Entry values value is not a string keep||",
            "!Cannot create entry with empty source text and no key. keep||",
            "!Entry had invalid source \"{ }\": \"{ }\". Invalid parameter name \" \". Parameter names can only contain letters (A-Z, a-z), digits (0-9), and underscores (_). keep||",
            "e1 v e !Contents is not a valid member of LocalizationTable \"LocalizationTable\"",
        ]
    );
}

/// What a Translator is (a new instance each call, named "Instance", locked parent, read-only LocaleId, no copy) and
/// LocalizationService's other answers, the same on a server; Roblox's own members raise
#[test]
fn translators_and_the_service_answer_as_a_client_does() {
    let source = r#"
        for _, Child in LS:GetChildren() do
            Child:Destroy()
        end
        local L = Instance.new("LocalizationTable")
        local T = L:GetTranslator("ES_es")
        print(T.ClassName, T.Name, T.LocaleId, T.Parent, T == L:GetTranslator("es-es"), T:Clone(), T:IsA("Translator"))
        print(Try(function() T.Parent = workspace end), Try(function() T.Parent = nil end), Try(function() T.LocaleId = "x" end), Try(Instance.new, "Translator"))
        print(Try(L.GetTranslator, L), L:GetTranslator(5).LocaleId, L:GetTranslator("").LocaleId, Try(T.FormatByKey, T), Try(T.FormatByKey, workspace, "k"))
        print(LS.RobloxLocaleId, LS.SystemLocaleId, Try(function() LS.RobloxLocaleId = "x" end))
        print(Try(LS.GetTranslatorForLocaleAsync, LS, "e"), LS:GetTranslatorForLocaleAsync("EN-US").LocaleId, Try(LS.GetTranslatorForLocaleAsync, LS))
        print(Try(LS.GetTranslatorForPlayer, LS, nil), Try(LS.GetTranslatorForPlayer, LS, workspace), Try(LS.GetTranslatorForPlayer, LS, 5))
        print(Try(LS.GetTranslatorForPlayerAsync, LS, nil), Try(LS.GetTranslatorForPlayerAsync, LS, workspace), #LS:GetCorescriptLocalizations(), #LS:GetTableEntries(nil))
        print(Try(T.RobloxOnlyTranslate, T, game, "x"), Try(LS.StartTextScraper, LS), Try(L.SetIsExemptFromUGCAnalytics, L, true))
        L.DevelopmentLanguage = "FR_ca"
        print(L.SourceLocaleId, L.DevelopmentLanguage)
        T:Destroy()
        print(T.LocaleId, Try(T.FormatByKey, T, "k"))
    "#;
    let expected = [
        "Translator Instance es-es nil false nil true",
        "!Cannot change Parent of type Translator !Cannot change Parent of type Translator !Unable to assign property LocaleId. Property is read only !Unable to create an Instance of type \"Translator\"",
        "!Argument 1 missing or nil 5  !Argument 1 missing or nil !Expected ':' not '.' calling member function FormatByKey",
        "en-us en-us !Unable to assign property RobloxLocaleId. Property is read only",
        "!Too few characters in locale id! en-us !Argument 1 missing or nil",
        "!Player cannot be nil !Instance was not a player !Unable to cast value to Object",
        "!Argument 1 missing or nil !Publishing the game is required to use GetTranslatorForPlayerAsync API. 0 1",
        "!The current thread cannot call 'RobloxOnlyTranslate' (lacking capability RobloxScript) !The current thread cannot call 'StartTextScraper' (lacking capability RobloxScript) !The current thread cannot call 'SetIsExemptFromUGCAnalytics' (lacking capability RobloxScript)",
        "fr-ca fr-ca",
        "es-es !Key k not found for locale es-es",
    ];
    assert_eq!(printed(source), expected);
    assert_eq!(printed_with(server(), source), expected);
}

/// The local player's translator reads the service's tables in its player's locale (an offline client's is ""), and with a
/// GameId GetTranslatorForPlayerAsync gives one too (Roblox's documentation: the measured place has no GameId, and raises)
#[test]
fn a_players_translator_has_its_locale() {
    let source = r#"
        local Player = game:GetService("Players").LocalPlayer
        local T = LS:GetTranslatorForPlayer(Player)
        print(T.LocaleId == Player.LocaleId, Try(T.FormatByKey, T, "k"), LS:GetTranslatorForPlayerAsync(Player).ClassName, LS:GetCountryRegionForPlayerAsync(Player))
    "#;
    assert_eq!(printed_with(Options { game_id: 1.0, ..Options::default() }, source), ["true !Key k not found for locale  Translator US"]);
}

/// GetEntries, SetEntries and SetContents write no Changed; SourceLocaleId does
#[test]
fn entry_writes_fire_no_changed() {
    let lines = printed(
        r#"
        local L = Instance.new("LocalizationTable")
        local Seen = {}
        L.Changed:Connect(function(Name) table.insert(Seen, Name) end)
        L:SetEntries({ V("k", "", "") })
        L:SetEntry("k", "es-es", "v")
        L:SetContents("[]")
        L.SourceLocaleId = "fr-fr"
        task.wait()
        print(table.concat(Seen, ","))
        "#,
    );
    assert_eq!(lines, ["SourceLocaleId"]);
}

#[test]
fn templates_parse_and_numbers_write_as_printf_does() {
    use super::{Format, Param, Piece, fixed, parse_template};
    assert_eq!(
        parse_template(b"x{{1}}{2:int}}}").unwrap(),
        [Piece::Text(b"x{1}".to_vec()), Piece::Param(Param::Index(2), Format::Int), Piece::Text(b"}".to_vec())]
    );
    assert_eq!(parse_template(b"{}").unwrap(), []);
    assert_eq!(fixed(0.125, 2), "0.12");
    assert_eq!(fixed(-1e-9, 0), "-0");
    assert_eq!(fixed(f64::NEG_INFINITY, 2), "-inf");
    assert_eq!(fixed(f64::from_bits(0xfff8_0000_0000_0000), 2), "-nan(ind)");
    assert_eq!(super::formats::number(b"es-419", -1234567.891), "-1.234.567,89");
    assert_eq!(super::formats::number(b"xx", 1e20), "100,000,000,000,000,000,000.00");
}

/// The work clock's prices (cost.rs LOCALIZATION_* and method): what a 0.747 client spent, each within its measured range (two
/// sessions: SetEntries of 100 entries with two values 259-295 us, GetEntries 175-213, FormatByKey of a key with no parameters
/// 0.36-0.64, with two named arguments one a number 2.3-3.2, Translate 9.1-9.7 on a loaded machine). ree charged 0.07 us for
/// each before.
#[test]
fn localization_calls_cost_what_a_client_spends() {
    let lines = printed(
        r#"
        local List = {}
        for I = 1, 100 do
            List[I] = { Key = "Key" .. I, Source = "Source text " .. I, Values = { ["es-es"] = "Texto " .. I, ["fr-fr"] = "Texte " .. I } }
        end
        local L = Instance.new("LocalizationTable")
        L:SetEntries(List)
        local Formats = Instance.new("LocalizationTable")
        Formats:SetEntries(List)
        Formats:SetEntry("Named", "es-es", "Hola {name}, tienes {count} monedas")
        local T = Formats:GetTranslator("es-es")
        task.wait()
        local function Per(Count, Body)
            local Start = os.clock()
            for _ = 1, Count do
                Body()
            end
            return (os.clock() - Start) / Count * 1e6
        end
        local Empty = Per(100, function() end)
        local Set = Per(10, function() L:SetEntries(List) end) - Empty
        local Got = Per(10, function() L:GetEntries() end) - Empty
        local Plain = Per(100, function() T:FormatByKey("Key50") end) - Empty
        local Named = Per(100, function() T:FormatByKey("Named", { name = "Ann", count = 5 }) end) - Empty
        local Translated = Per(100, function() T:Translate(game, "Source text 5") end) - Empty
        print(Set >= 230 and Set <= 330, Got >= 150 and Got <= 240, Plain >= 0.3 and Plain <= 0.7, Named >= 1.8 and Named <= 3.4, Translated >= 7 and Translated <= 10)
        print(string.format("%.1f %.1f %.2f %.2f %.2f", Set, Got, Plain, Named, Translated))
        "#,
    );
    assert_eq!(lines[0], "true true true true true", "{lines:?}");
}
