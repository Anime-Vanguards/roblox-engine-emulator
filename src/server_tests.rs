//! The server run context (`--run-context server`, lua/server.luau) through the script API: RunService's
//! answers and no local player, the in-memory DataStoreService, MemoryStoreService and MessagingService,
//! Player:Kick and the server side of remotes; EncodingService, and a client that keeps its stubs.

use std::rc::Rc;

use mlua::prelude::*;

use super::{
    Engine, Options, RunContext,
    lua::{self as engine_lua, globals::CallOutcome},
    scheduler,
};

#[inline(never)]
fn start(run_context: RunContext) -> (Lua, Rc<Engine>) {
    let options = match run_context {
        RunContext::Server => Options { mode: "play".into(), run_context, local_player: false, ..Options::default() },
        RunContext::Client | RunContext::Loopback => Options { mode: "play".into(), ..Options::default() },
        RunContext::Edit => Options { run_context, ..Options::default() },
    };
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    (lua, engine)
}

/// Runs `source` as a script of a fresh engine, then its clock to `until`: the output lines ("kind: message"),
/// none of which may be an error.
#[inline(never)]
fn run(run_context: RunContext, source: &str, until: f64) -> Vec<String> {
    let (lua, engine) = start(run_context);
    run_in(&lua, &engine, source, until)
}

#[inline(never)]
fn run_in(lua: &Lua, engine: &Rc<Engine>, source: &str, until: f64) -> Vec<String> {
    run_with(lua, engine, source, until, &[])
}

/// [`run_in`] with `globals` set in the script's environment.
#[inline(never)]
fn run_with(lua: &Lua, engine: &Rc<Engine>, source: &str, until: f64, globals: &[(&str, LuaValue)]) -> Vec<String> {
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        super::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(lua, engine, script).expect("environment");
    for (name, value) in globals {
        env.set(*name, value.clone()).expect("global");
    }
    let function = lua.load(source).set_name("=Test").set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(lua, engine, function, LuaMultiValue::new()).expect("call");
    if let CallOutcome::Failed(message, _) = call.outcome().unwrap() {
        panic!("test script failed: {message}");
    }
    let now = engine.state.borrow().sched.now;
    scheduler::run_until(lua, engine, now + until).unwrap();
    // (then to the script's end: what follows a wait past `until` is checked too)
    if let Err(message) = call.finish(lua, engine) {
        panic!("test script failed: {message}");
    }
    let lines: Vec<String> = engine.state.borrow().output.entries.iter().map(|e| format!("{}: {}", e.kind, e.message)).collect();
    let errors: Vec<&String> = lines.iter().filter(|line| line.starts_with("error")).collect();
    assert!(errors.is_empty(), "{errors:?}");
    lines
}

#[inline(never)]
fn has(lines: &[String], wanted: &str) -> bool {
    lines.iter().any(|line| line == wanted)
}

/// A printed line `label` then an error ending in `message` (after the script position, when it has one).
#[inline(never)]
fn has_error(lines: &[String], label: &str, message: &str) -> bool {
    let start = format!("print: {label} ");
    lines.iter().any(|line| line.starts_with(&start) && line.ends_with(message))
}

/// A server has no typesetting (measured on a Studio 0.747 server, tools/parity/probes/text_bounds.luau): GetTextSize answers (0, 0)
/// whatever it is asked, and GetTextBoundsAsync raises once its argument is a GetTextBoundsParams. ree measured text there as a client.
#[test]
fn a_server_measures_no_text() {
    let lines = run(
        RunContext::Server,
        r#"
        local TextService = game:GetService("TextService")
        print("size", TextService:GetTextSize("Hello world", 20, Enum.Font.SourceSans, Vector2.new(1e5, 1e5)))
        print("bounds", pcall(TextService.GetTextBoundsAsync, TextService, Instance.new("GetTextBoundsParams")))
        print("nil", pcall(TextService.GetTextBoundsAsync, TextService, nil))
        "#,
        0.1,
    );
    assert!(has(&lines, "print: size 0, 0"), "{lines:?}");
    assert!(has(&lines, "print: bounds false Typesetting is not enabled in this DataModel"), "{lines:?}");
    assert!(has(&lines, "print: nil false Argument 1 missing or nil"), "{lines:?}");
}

#[test]
fn a_server_has_no_local_player_and_renders_nothing() {
    let lines = run(
        RunContext::Server,
        r#"
        local RunService = game:GetService("RunService")
        local Players = game:GetService("Players")
        assert(RunService:IsServer() and not RunService:IsClient() and RunService:IsStudio() and RunService:IsRunning())
        assert(not RunService:IsRunMode())
        -- (measured on a Studio 0.744 server, tools/parity/probes/run_service_sides.luau: a game script may not call IsEdit
        -- nor connect to RenderStepped; PreRender and BindToRenderStep are taken and never run)
        print("is-edit", pcall(RunService.IsEdit, RunService))
        print("render-stepped", pcall(function() RunService.RenderStepped:Connect(function() end) end))
        assert(Players.LocalPlayer == nil and #Players:GetPlayers() == 0)
        local Rendered, Bound, Beats = 0, 0, 0
        RunService.PreRender:Connect(function() Rendered += 1 end)
        RunService:BindToRenderStep("Test", 1, function() Bound += 1 end)
        RunService.Heartbeat:Connect(function() Beats += 1 end)
        Players.PlayerAdded:Connect(function(Player)
            print("added", Player.Name, Player.UserId)
            local Names = {}
            for _, Child in Player:GetChildren() do
                table.insert(Names, Child.ClassName)
            end
            table.sort(Names)
            print("containers", table.concat(Names, ","))
            -- (measured, Studio 0.747 server: PlayerGui is locked by class, the Backpack not at all)
            print("locks", select(2, pcall(function() Player.PlayerGui.Parent = Player end)), pcall(function() Player.Backpack.Parent = Player end))
        end)
        Players.PlayerRemoving:Connect(function(Player) print("removing", Player.Name) end)
        local Player = Instance.new("Player")
        Player.Name = "Fake"
        Player.UserId = 7
        Player.Parent = Players
        task.wait(0.1)
        print("frames", Rendered, Bound, Beats > 3, #Players:GetPlayers())
        Player:Kick("bye")
        print("kicked", Player.Parent == Players)
        task.wait(1.1)
        print("left", Player.Parent)
        "#,
        0.5,
    );
    for wanted in [
        "print: is-edit false The current thread cannot call 'IsEdit' (lacking capability PluginOrOpenCloud)",
        "print: render-stepped false RenderStepped event can only be used from local scripts",
        "print: added Fake 7",
        "print: containers Backpack,PlayerGui,StarterGear",
        "print: locks Cannot change Parent of type PlayerGui true",
        "print: frames 0 0 true 1",
        "print: kicked true",
        "print: removing Fake",
        "print: left nil",
    ] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

#[test]
fn data_stores_keep_copies_versions_and_metadata() {
    let (lua, engine) = start(RunContext::Server);
    let lines = run_in(
        &lua,
        &engine,
        r#"
        local DataStoreService = game:GetService("DataStoreService")
        local Store = DataStoreService:GetDataStore("Profiles")
        assert(DataStoreService:GetDataStore("Profiles") == Store and Store.Name == "Profiles" and Store:IsA("GlobalDataStore"))
        assert(DataStoreService:GetRequestBudgetForRequestType(Enum.DataStoreRequestType.UpdateAsync) == 60)
        local Before = time()
        local Value = { Coins = 5, Items = { "a", "b" }, Blob = buffer.fromstring("zz") }
        local Options = Instance.new("DataStoreSetOptions")
        Options:SetMetadata({ Tag = "x" })
        local Version = Store:SetAsync("1", Value, { 1 }, Options)
        assert(time() > Before, "SetAsync yields a frame")
        Value.Coins = 6
        local Read, Info = Store:GetAsync("1")
        assert(Read.Coins == 5 and Read.Items[2] == "b" and buffer.tostring(Read.Blob) == "zz", "a copy of what was written")
        assert(Info:IsA("DataStoreKeyInfo") and Info.Version == Version and Info:GetMetadata().Tag == "x" and Info:GetUserIds()[1] == 1)
        local Updated, UpdatedInfo = Store:UpdateAsync("1", function(Old, OldInfo)
            assert(OldInfo.Version == Version)
            Old.Coins += 10
            return Old, { 2 }, { Tag = "y" }
        end)
        assert(Updated.Coins == 15 and UpdatedInfo.Version ~= Version and UpdatedInfo:GetUserIds()[1] == 2)
        assert(UpdatedInfo.CreatedTime == Info.CreatedTime and UpdatedInfo:GetMetadata().Tag == "y")
        assert(Store:UpdateAsync("1", function() return nil end) == nil and Store:GetAsync("1").Coins == 15, "a nil transform cancels")
        assert(Store:GetVersionAsync("1", Version).Coins == 5)
        print("increment", Store:IncrementAsync("n", 3), Store:IncrementAsync("n"))
        local Removed = Store:RemoveAsync("1")
        assert(Removed.Coins == 15 and Store:GetAsync("1") == nil)
        local Versions = Store:ListVersionsAsync("1"):GetCurrentPage()
        print("versions", #Versions, Versions[1].IsDeleted, Versions[3].IsDeleted)
        local Keys = Store:ListKeysAsync("", 0, "", true):GetCurrentPage()
        print("keys", #Keys, Keys[1].KeyName)
        for _, Bad in {
            function() Store:GetAsync("") end,
            function() Store:GetAsync(string.rep("k", 51)) end,
            function() Store:SetAsync("f", print) end,
            function() Store:SetAsync("t", { 1, x = 2 }) end,
            function() Store:SetAsync("big", string.rep("x", 4194304)) end,
            function() Store:UpdateAsync("y", function() task.wait() return 1 end) end,
        } do
            print("error", (select(2, pcall(Bad))))
        end
        "#,
        1.0,
    );
    for wanted in ["print: increment 3 4", "print: versions 3 false true", "print: keys 1 n"] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
    for wanted in [
        "Test:33: 101: Key name can't be empty.",
        "Test:34: 102: Key name exceeds the 50 character limit.",
        "Test:35: 104: Cannot store function in data store. Data stores can only accept valid UTF-8 characters.",
        "Test:36: 104: Cannot store Array in data store. Data stores can only accept valid UTF-8 characters.",
        "Test:37: 105: Serialized value exceeds 4MB limit.",
        "UpdateAsync: the transform function cannot yield",
    ] {
        assert!(has(&lines, &format!("print: error {wanted}")), "{wanted}: {lines:?}");
    }
    // another run's stores start empty
    let other = run(RunContext::Server, r#"print("other", game:GetService("DataStoreService"):GetDataStore("Profiles"):GetAsync("n"))"#, 0.1);
    assert!(has(&other, "print: other nil"), "{other:?}");
}

/// Data stores, memory stores and MessagingService convert a value as an event copies a table (lua/bindable.rs): a table
/// that would sit above the Lua stack's slot 300 raises "Max lua stack size reached" and a table inside itself "tables
/// cannot be cyclic", without a position and before anything that cannot be stored, wherever it is. The outermost table
/// sits at 1 for SetAsync and AddAsync, 6 for an UpdateAsync transform's result, 4 for PublishAsync (measured in a 0.747
/// playing server, tools/parity/probes/table_depth.luau); 100,000 levels raise the same error. A message's size counts
/// its JSON's separators, one fewer than its entries.
#[test]
fn stores_refuse_tables_nested_past_the_lua_stack() {
    let lines = run(
        RunContext::Server,
        r#"
        local function Nest(Depth, Key)
            local Root = {}
            local At = Root
            for _ = 2, Depth do
                local Inner = {}
                At[Key or 1] = Inner
                At = Inner
            end
            return Root
        end
        local function Ring(Depth)
            local Root = Nest(Depth)
            local At = Root
            while At[1] do
                At = At[1]
            end
            At[1] = Root
            return Root
        end
        local function Try(Name, Call)
            print(Name, pcall(Call))
        end
        local Store = game:GetService("DataStoreService"):GetDataStore("Depth")
        local Memory = game:GetService("MemoryStoreService")
        local Map, Hash, Queue = Memory:GetSortedMap("Depth"), Memory:GetHashMap("Depth"), Memory:GetQueue("Depth")
        local Messaging = game:GetService("MessagingService")
        Try("set 300", function() Store:SetAsync("k", Nest(300)) end)
        Try("set 301", function() Store:SetAsync("k", Nest(301)) end)
        Try("set dict 150", function() Store:SetAsync("k", Nest(150, "a")) end)
        Try("set dict 151", function() Store:SetAsync("k", Nest(151, "a")) end)
        Try("update 295", function() Store:UpdateAsync("k", function() return Nest(295) end) end)
        Try("update 296", function() Store:UpdateAsync("k", function() return Nest(296) end) end)
        Try("sorted 300", function() Map:SetAsync("k", Nest(300), 30) end)
        Try("sorted 301", function() Map:SetAsync("k", Nest(301), 30) end)
        Try("sorted update 296", function() Map:UpdateAsync("k", function() return Nest(296) end, 30) end)
        Try("hash 301", function() Hash:SetAsync("k", Nest(301), 30) end)
        Try("hash update 296", function() Hash:UpdateAsync("k", function() return Nest(296) end, 30) end)
        Try("queue 300", function() Queue:AddAsync(Nest(300), 30) end)
        Try("queue 301", function() Queue:AddAsync(Nest(301), 30) end)
        Try("publish 297", function() Messaging:PublishAsync("t", Nest(297)) end)
        Try("publish 298", function() Messaging:PublishAsync("t", Nest(298)) end)
        Try("publish dict 149", function() Messaging:PublishAsync("t", Nest(149, "a")) end)
        Try("publish dict 150", function() Messaging:PublishAsync("t", Nest(150, "a")) end)
        Try("set ring", function() Store:SetAsync("k", Ring(3)) end)
        Try("set function then ring", function() Store:SetAsync("k", { print, Ring(3) }) end)
        Try("set function then deep", function() Store:SetAsync("k", { print, Nest(400) }) end)
        Try("set ring 300", function() Store:SetAsync("k", Ring(300)) end)
        Try("publish ring", function() Messaging:PublishAsync("t", Ring(3)) end)
        Try("set 100000", function() Store:SetAsync("k", Nest(100000)) end)
        Try("publish 100000", function() Messaging:PublishAsync("t", Nest(100000)) end)
        Try("queue 100000", function() Queue:AddAsync(Nest(100000), 30) end)
        "#,
        3.0,
    );
    let deep = "Max lua stack size reached";
    for label in ["set 300", "set dict 150", "update 295", "sorted 300", "queue 300", "publish 297", "publish dict 149"] {
        assert!(has(&lines, &format!("print: {label} true")), "{label}: {lines:?}");
    }
    for label in [
        "set 301", "set dict 151", "update 296", "sorted 301", "sorted update 296", "hash 301", "hash update 296", "queue 301", "publish 298",
        "publish dict 150", "set function then deep", "set ring 300", "set 100000", "publish 100000", "queue 100000",
    ] {
        assert!(has(&lines, &format!("print: {label} false {deep}")), "{label}: {lines:?}");
    }
    for label in ["set ring", "set function then ring", "publish ring"] {
        assert!(has(&lines, &format!("print: {label} false tables cannot be cyclic")), "{label}: {lines:?}");
    }
}

/// A store converts a value as an event copies it, a table once for each place it appears (lua/bindable.rs), its time
/// growing with what it writes (a 0.747 server: SetAsync of one table of 1,000 numbers held 8,000 times took 0.4 s, and a
/// cycle after all of it still raised first). ree stops copying once the value's JSON is longer than the store takes, goes
/// on looking for the errors that win over its size, and stops looking past its guard on a copy (bindable.rs
/// `COPY_VALUES`) with the store's size error: the same 100,000 trues held 100,000 times (10^10 values without the guard)
/// raise each store's size error, a cycle after 5 MB of them is still found, and a function before them still gives the 104
/// error once the walk for a cycle stops at the guard. A value below the limits is stored whole, a table for each place.
#[test]
fn stores_stop_at_rees_guard_on_a_table_converted_for_each_place_it_appears() {
    let prelude = r#"
        local function Try(Name, Call)
            print(Name, pcall(Call))
        end
        local Store = game:GetService("DataStoreService"):GetDataStore("Breadth")
        local Map = game:GetService("MemoryStoreService"):GetSortedMap("Breadth")
        local Messaging = game:GetService("MessagingService")
    "#;
    let error_cases = [
        ("set huge", "local Huge = table.create(100000, table.create(100000, true))\nTry(\"set huge\", function() Store:SetAsync(\"k\", Huge) end)", "105: Serialized value exceeds 4MB limit."),
        ("update huge", "local Huge = table.create(100000, table.create(100000, true))\nTry(\"update huge\", function() Store:UpdateAsync(\"k\", function() return Huge end) end)", "105: Serialized value exceeds 4MB limit."),
        ("sorted huge", "local Huge = table.create(100000, table.create(100000, true))\nTry(\"sorted huge\", function() Map:SetAsync(\"k\", Huge, 30) end)", "Code: ItemValueSizeTooLarge, Error: Value size exceeds limit (32 KB)."),
        ("publish huge", "local Huge = table.create(100000, table.create(100000, true))\nTry(\"publish huge\", function() Messaging:PublishAsync(\"t\", Huge) end)", "PublishAsync: the message is larger than 1 kB"),
        ("set big", "local Big = table.create(1000, table.create(1000, true))\nTry(\"set big\", function() Store:SetAsync(\"k\", Big) end)", "105: Serialized value exceeds 4MB limit."),
        ("set big then ring", "local Big = table.create(1000, table.create(1000, true))\nlocal Ring = {}\nRing[1] = Ring\nTry(\"set big then ring\", function() Store:SetAsync(\"k\", { Big, Ring }) end)", "tables cannot be cyclic"),
        ("set function then huge", "local Huge = table.create(100000, table.create(100000, true))\nTry(\"set function then huge\", function() Store:SetAsync(\"k\", { print, Huge }) end)", "104: Cannot store function in data store. Data stores can only accept valid UTF-8 characters."),
    ];
    std::thread::scope(|s| {
        let handles: Vec<_> = error_cases
            .into_iter()
            .map(|(label, snippet, message)| {
                s.spawn(move || {
                    let lines = run(RunContext::Server, &format!("{prelude}\n{snippet}\n"), 3.0);
                    assert!(has_error(&lines, &format!("{label} false"), message), "{label}: {lines:?}");
                })
            })
            .collect();
        let fan = s.spawn(|| {
            let lines = run(
                RunContext::Server,
                &format!(
                    "{prelude}\nTry(\"set fan\", function() Store:SetAsync(\"k\", table.create(100, table.create(1000, true))) end)\nlocal Read = Store:GetAsync(\"k\")\nprint(\"read\", #Read, #Read[100], Read[1] ~= Read[2])\n"
                ),
                3.0,
            );
            assert!(has(&lines, "print: set fan true"), "{lines:?}");
            assert!(has(&lines, "print: read 100 1000 true"), "{lines:?}");
        });
        for handle in handles {
            handle.join().unwrap();
        }
        fan.join().unwrap();
    });
}

#[test]
fn ordered_data_stores_page_by_value() {
    let lines = run(
        RunContext::Server,
        r#"
        local Board = game:GetService("DataStoreService"):GetOrderedDataStore("Board")
        for Index, Score in { 30, 10, 50, 20, 40 } do
            Board:SetAsync("p" .. Index, Score)
        end
        local Pages = Board:GetSortedAsync(false, 2)
        local First = Pages:GetCurrentPage()
        print("first", First[1].key, First[1].value, First[2].value, Pages.IsFinished)
        Pages:AdvanceToNextPageAsync()
        Pages:AdvanceToNextPageAsync()
        print("last", #Pages:GetCurrentPage(), Pages.IsFinished, (select(2, pcall(Pages.AdvanceToNextPageAsync, Pages))))
        print("ranged", #Board:GetSortedAsync(true, 100, 20, 40):GetCurrentPage())
        print("bad", (select(2, pcall(Board.GetSortedAsync, Board, true, 0))))
        "#,
        1.0,
    );
    for wanted in ["print: first p3 50 40 false", "print: last 1 true Test:11: No pages to advance to.", "print: ranged 3", "print: bad Test:13: 106: PageSize must be within a predefined range."] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

#[test]
fn memory_stores_order_expire_and_hide_read_items() {
    let lines = run(
        RunContext::Server,
        r#"
        local MemoryStoreService = game:GetService("MemoryStoreService")
        local Map = MemoryStoreService:GetSortedMap("Ranks")
        print("overwrote", Map:SetAsync("b", 1, 60, 5), Map:SetAsync("b", 2, 60, 5))
        Map:SetAsync("a", 3, 60, 5)
        Map:SetAsync("c", 4, 60, "s")
        Map:SetAsync("d", 5, 60)
        Map:SetAsync("e", 6, 60, -1)
        local Order = {}
        for _, Item in Map:GetRangeAsync(Enum.SortDirection.Ascending, 10) do
            table.insert(Order, Item.key)
        end
        print("order", table.concat(Order, ","))
        local After = {}
        for _, Item in Map:GetRangeAsync(Enum.SortDirection.Descending, 2, { key = "a", sortKey = 5 }) do
            table.insert(After, Item.key)
        end
        print("after", table.concat(After, ","))
        print("update", Map:UpdateAsync("a", function(Value, Sort) return Value + 1, Sort end, 60))
        local Hash = MemoryStoreService:GetHashMap("Cache")
        Hash:SetAsync("k", { 1 }, 1)
        print("hash", Hash:GetAsync("k")[1])
        task.wait(1.05)
        print("expired", Hash:GetAsync("k"), Map:GetSizeAsync())
        local Queue = MemoryStoreService:GetQueue("Jobs", 0.5)
        Queue:AddAsync("low", 60)
        Queue:AddAsync("high", 60, 5)
        local Items, Id = Queue:ReadAsync(1)
        print("read", Items[1], Queue:GetSizeAsync(), Queue:GetSizeAsync(true))
        Queue:RemoveAsync(Id)
        print("removed", Queue:GetSizeAsync())
        print("hidden", Queue:ReadAsync(1)[1], #Queue:ReadAsync(1, false, 0))
        task.wait(0.6)
        print("visible", Queue:ReadAsync(1)[1])
        print("bad", (select(2, pcall(Map.SetAsync, Map, "x", 1, 4000000))))
        "#,
        3.0,
    );
    for wanted in [
        "print: overwrote false true",
        "print: order e,a,b,c,d",
        "print: after d,c",
        "print: update 4 5",
        "print: hash 1",
        "print: expired nil 5",
        "print: read high 2 1",
        "print: removed 1",
        "print: hidden low 0",
        "print: visible low",
        "print: bad Test:35: Code: InvalidExpirationTime, Error: The field 'expiration' time must be between 0 and 3,888,000.",
    ] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

#[test]
fn messages_reach_subscribers_and_remotes_reach_no_client() {
    let lines = run(
        RunContext::Server,
        r#"
        local MessagingService = game:GetService("MessagingService")
        local Got = {}
        local Connection = MessagingService:SubscribeAsync("Topic", function(Message)
            table.insert(Got, `{Message.Data.N}:{type(Message.Sent)}`)
        end)
        MessagingService:PublishAsync("Topic", { N = 1 })
        task.wait()
        Connection:Disconnect()
        MessagingService:PublishAsync("Topic", { N = 2 })
        task.wait()
        print("messages", table.concat(Got, ","))
        local Players = game:GetService("Players")
        local Player = Instance.new("Player")
        Player.Parent = Players
        local Remote = Instance.new("RemoteEvent")
        Remote.OnServerEvent:Connect(function() end)
        Remote:FireClient(Player, 1)
        Remote:FireAllClients(2)
        Instance.new("UnreliableRemoteEvent"):FireAllClients(3)
        print("fire server", (select(2, pcall(Remote.FireServer, Remote))))
        print("fire client", (select(2, pcall(Remote.FireClient, Remote, workspace))))
        local Function = Instance.new("RemoteFunction")
        print("invoke server", (select(2, pcall(Function.InvokeServer, Function))))
        local Answered = false
        task.spawn(function()
            local Ok, Message = pcall(Function.InvokeClient, Function, Player, 1)
            Answered = true
            print("invoke client", Ok, Message)
        end)
        task.wait(0.5)
        print("waiting", Answered)
        Player.Parent = nil
        "#,
        1.0,
    );
    for wanted in [
        "print: messages 1:number",
        "print: fire server Test:21: FireServer can only be called from the client",
        "print: fire client Test:22: FireClient: player argument must be a Player object",
        "print: invoke server Test:24: InvokeServer can only be called from the client",
        "print: waiting false",
        "print: invoke client false Player Player disconnected during remote call to RemoteFunction",
    ] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

/// A client keeps what it had: its local player and no data stores (their members are not emulated there, Output
/// `missing`).
#[test]
fn a_client_keeps_its_stubs() {
    let lines = run(
        RunContext::Client,
        r#"
        local RunService = game:GetService("RunService")
        print("client", RunService:IsClient(), RunService:IsServer(), game.Players.LocalPlayer.Name, select('#', game:GetService("DataStoreService"):GetDataStore("X")))
        "#,
        0.1,
    );
    assert!(has(&lines, "print: client true false Player 0"), "{lines:?}");
}

/// EncodingService answers buffers on a playing client as on a server, with the client's errors (measured in a 0.747
/// client and server: tools/parity/probes/encoding.luau).
#[test]
fn clients_and_servers_compress_and_encode_buffers() {
    for run_context in [RunContext::Client, RunContext::Server] {
        let lines = run(
            run_context,
            r#"
        local EncodingService = game:GetService("EncodingService")
        local Text = string.rep("abc", 1000)
        local Packed = EncodingService:CompressBuffer(buffer.fromstring(Text), Enum.CompressionAlgorithm.Zstd, 4)
        assert(buffer.len(Packed) < 100)
        assert(EncodingService:GetDecompressedBufferSize(Packed, Enum.CompressionAlgorithm.Zstd) == 3000)
        assert(buffer.tostring(EncodingService:DecompressBuffer(Packed, Enum.CompressionAlgorithm.Zstd)) == Text)
        assert(buffer.tostring(EncodingService:Base64Encode(buffer.fromstring("hi!"))) == "aGkh")
        assert(buffer.tostring(EncodingService:Base64Decode(buffer.fromstring("aGkh"))) == "hi!")
        print("bad", pcall(EncodingService.DecompressBuffer, EncodingService, buffer.fromstring("nope"), Enum.CompressionAlgorithm.Zstd))
        print("level", pcall(EncodingService.CompressBuffer, EncodingService, buffer.fromstring("x"), Enum.CompressionAlgorithm.Zstd, 23))
        print("size", EncodingService:GetDecompressedBufferSize(buffer.fromstring("nope"), Enum.CompressionAlgorithm.Zstd))
        print("unpadded", pcall(EncodingService.Base64Decode, EncodingService, buffer.fromstring("aGVsbG8gd29ybGQ")))
        print("string", pcall(EncodingService.Base64Encode, EncodingService, "hi!"))
        "#,
            0.1,
        );
        assert!(has(&lines, "print: size nil"), "{run_context:?}: {lines:?}");
        assert!(has_error(&lines, "bad false", "DecompressBuffer: decompression failed - Unspecified error code"), "{run_context:?}: {lines:?}");
        assert!(has_error(&lines, "level false", "CompressBuffer: unsupported compression level for Zstd"), "{run_context:?}: {lines:?}");
        assert!(has_error(&lines, "unpadded false", "Base64Decode: input is not a base64 string"), "{run_context:?}: {lines:?}");
        assert!(has_error(&lines, "string false", "Argument 1 missing or nil"), "{run_context:?}: {lines:?}");
    }
}

/// EncodingService:ComputeBufferHash and ComputeStringHash on a playing client and a server alike (measured in a 0.747 client and
/// server, the same lines on both: tools/parity/probes/encoding_hash.luau): each HashAlgorithm's digest, as a buffer and as a string
/// of its bytes, Blake2b's 32 bytes long; the algorithm is cast before the input (an item, its exact name, or its value cut toward
/// zero), then the input, a string parameter's (a boolean warns and is hashed as its text). They answered nil and reported
/// themselves `missing`.
#[test]
fn clients_and_servers_hash_buffers_and_strings() {
    for run_context in [RunContext::Client, RunContext::Server] {
        let lines = run(
            run_context,
            r#"
        local EncodingService = game:GetService("EncodingService")
        local function hex(Text)
            return (string.gsub(Text, ".", function(Byte)
                return string.format("%02x", string.byte(Byte))
            end))
        end
        for _, Algorithm in Enum.HashAlgorithm:GetEnumItems() do
            local Digest = EncodingService:ComputeBufferHash(buffer.fromstring("abc"), Algorithm)
            local Text = EncodingService:ComputeStringHash("abc", Algorithm)
            print(Algorithm.Name, typeof(Digest), typeof(Text), Text == buffer.tostring(Digest), hex(Text))
        end
        print("empty", hex(buffer.tostring(EncodingService:ComputeBufferHash(buffer.create(0), Enum.HashAlgorithm.Blake3))))
        print("casts", hex(EncodingService:ComputeStringHash("abc", 2.5)), hex(EncodingService:ComputeStringHash("abc", "Sha1")))
        print("boolean", hex(EncodingService:ComputeStringHash(true, Enum.HashAlgorithm.Md5)))
        print("none", pcall(EncodingService.ComputeBufferHash, EncodingService))
        print("string", pcall(EncodingService.ComputeBufferHash, EncodingService, "abc", Enum.HashAlgorithm.Md5))
        print("zstd", pcall(EncodingService.ComputeStringHash, EncodingService, "abc", Enum.CompressionAlgorithm.Zstd))
        print("enum", pcall(EncodingService.ComputeStringHash, EncodingService, "abc", Enum.HashAlgorithm))
        print("lower", pcall(EncodingService.ComputeStringHash, EncodingService, "abc", "sha1"))
        print("first", pcall(EncodingService.ComputeStringHash, EncodingService, true, "Nope"))
        "#,
            0.1,
        );
        for wanted in [
            "print: Blake2b buffer string true bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319",
            "print: Blake3 buffer string true 6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
            "print: Md5 buffer string true 900150983cd24fb0d6963f7d28e17f72",
            "print: Sha1 buffer string true a9993e364706816aba3e25717850c26c9cd0d89d",
            "print: Sha256 buffer string true ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "print: empty af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            "print: casts 900150983cd24fb0d6963f7d28e17f72 a9993e364706816aba3e25717850c26c9cd0d89d",
            "print: boolean b326b5062b2f0e69046810717534cb09",
        ] {
            assert!(has(&lines, wanted), "{run_context:?}: {wanted}: {lines:?}");
        }
        assert!(has_error(&lines, "none false", "Argument 2 missing or nil"), "{run_context:?}: {lines:?}");
        assert!(has_error(&lines, "string false", "Argument 1 missing or nil"), "{run_context:?}: {lines:?}");
        assert!(has_error(&lines, "zstd false", "Unable to cast CompressionAlgorithm to HashAlgorithm"), "{run_context:?}: {lines:?}");
        assert!(has_error(&lines, "enum false", "Unable to cast null to HashAlgorithm"), "{run_context:?}: {lines:?}");
        assert!(has_error(&lines, "lower false", "Unable to cast string to HashAlgorithm"), "{run_context:?}: {lines:?}");
        assert!(has_error(&lines, "first false", "Unable to cast string to HashAlgorithm"), "{run_context:?}: {lines:?}");
        // (one warning: the boolean of the call whose algorithm is refused is never cast)
        let warned = lines.iter().filter(|line| line.starts_with("warn: argument #1 expects a string, but boolean was passed")).count();
        assert_eq!(warned, 1, "{run_context:?}: {lines:?}");
        assert!(!lines.iter().any(|line| line.starts_with("missing:")), "{run_context:?}: {lines:?}");
    }
}

/// A server's output reaches its scripts: LogService.MessageOut (message, MessageType) for each line and ScriptContext.Error
/// (message, stack trace, nil) for a script error, deferred. MessageOut gets them after the frame's Heartbeat, an error with
/// its stack lines, on a playing client and a server alike (measured: tools/parity/probes/logservice.luau, run.mjs and
/// run.mjs --server; a server passes the script to ScriptContext.Error, here nil); a playing client's ScriptContext.Error is
/// not emulated.
#[test]
fn a_server_hears_its_output_through_log_service_and_script_context() {
    for run_context in [RunContext::Server, RunContext::Client] {
        let (lua, engine) = start(run_context);
        let script = {
            let mut state = engine.state.borrow_mut();
            let class = engine.reflection.class_id("ModuleScript").unwrap();
            super::instance::create_named(&mut state, &engine.reflection, class, "Test")
        };
        let env = engine_lua::modules::environment(&lua, &engine, script).expect("environment");
        let source = r#"
            local Heard, Errors = {}, {}
            game:GetService("LogService").MessageOut:Connect(function(Message, Kind)
                table.insert(Heard, `{Kind.Name}={Message}`)
            end)
            game:GetService("ScriptContext").Error:Connect(function(Message, Trace, Script)
                table.insert(Errors, `{Message}|{Trace ~= ""}|{Script}`)
            end)
            print("one", 2)
            warn("careful")
            task.spawn(function() error("boom", 0) end)
            task.wait(0.05)
            print(`heard {table.concat(Heard, ";")}`)
            print(`errors {table.concat(Errors, ";")}`)
        "#;
        let function = lua.load(source).set_name("=Test").set_environment(env).into_function().expect("compile");
        let call = engine_lua::globals::call(&lua, &engine, function, LuaMultiValue::new()).expect("call");
        assert!(!matches!(call.outcome().unwrap(), CallOutcome::Failed(..)));
        let now = engine.state.borrow().sched.now;
        scheduler::run_until(&lua, &engine, now + 0.2).unwrap();
        call.finish(&lua, &engine).expect("the script runs to its end");
        let lines: Vec<String> = engine.state.borrow().output.entries.iter().map(|e| format!("{}: {}", e.kind, e.message)).collect();
        let heard = "print: heard MessageOutput=one 2;MessageWarning=careful;MessageError=boom\n    Stack Begin\n    Script 'Test', Line 11\n    Stack End";
        assert!(has(&lines, heard), "{run_context:?}: {lines:?}");
        assert!(has(&lines, "print: errors boom|true|Test"), "{run_context:?}: {lines:?}");
    }
}

/// A server's LogService.MessageOut gets its lines as a playing client's does (measured in a server game script, Studio 0.744):
/// after the frame's Heartbeat handlers, not when the line is logged, three arguments, the third a table (a LogService
/// method's converted context, an empty one for print); a line logged in a MessageOut handler reaches no handler. Its
/// LogService methods log and throw as a client's.
#[test]
fn a_servers_message_out_is_queued_with_a_context_argument() {
    let (lua, engine) = start(RunContext::Server);
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        super::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(&lua, &engine, script).expect("environment");
    let source = r##"
        local LogService = game:GetService("LogService")
        local Marks = {}
        LogService.MessageOut:Connect(function(...)
            local Message, Kind, Context = ...
            local Keys = {}
            for Key, Value in Context do
                table.insert(Keys, tostring(Key) .. "=" .. tostring(Value))
            end
            table.insert(Marks, Message .. ":" .. Kind.Name .. ":" .. select("#", ...) .. ":" .. table.concat(Keys, ";"))
            if Message == "outer" then
                LogService:Info("inner")
            end
        end)
        game:GetService("RunService").Heartbeat:Once(function()
            table.insert(Marks, "|hb")
        end)
        LogService:Info("i {a}", { a = 1 })
        table.insert(Marks, "after-info")
        print("p")
        local Ok, Thrown = pcall(LogService.Error, LogService, "e {a}", { a = 2 })
        table.insert(Marks, "after-error:" .. typeof(Thrown) .. ":" .. tostring(Thrown))
        LogService:Info("outer")
        task.wait(0.05)
        print("marks " .. table.concat(Marks, ","))
    "##;
    let function = lua.load(source).set_name("=Test").set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(&lua, &engine, function, LuaMultiValue::new()).expect("call");
    assert!(!matches!(call.outcome().unwrap(), CallOutcome::Failed(..)));
    let now = engine.state.borrow().sched.now;
    scheduler::run_until(&lua, &engine, now + 0.1).unwrap();
    call.finish(&lua, &engine).expect("the script runs to its end");
    let lines: Vec<String> = engine.state.borrow().output.entries.iter().map(|e| format!("{}: {}", e.kind, e.message)).collect();
    assert!(has(&lines, "print: marks after-info,after-error:table:e 2,|hb,i 1:MessageInfo:3:a=1,p:MessageOutput:3:,e 2:MessageError:3:a=2,outer:MessageInfo:3:"), "{lines:?}");
    // (the output keeps every line, the one no handler heard included)
    assert!(has(&lines, "info: i 1") && has(&lines, "error: e 2") && has(&lines, "info: inner"), "{lines:?}");
}

/// Offline a server makes the default R6 rig for Players:CreateHumanoidModelFromDescription(Async) with an R6 rig type (a
/// root part, a torso, a head and four limbs joined by the standard Motor6Ds, PrimaryPart the root), R15 a bare Model with
/// a Humanoid; InsertService:LoadAsset raises after a frame as a failed load does; Player:LoadCharacterAsync is a no-op.
#[test]
fn a_server_makes_r6_rigs_raises_on_load_asset_and_loads_no_character() {
    let lines = run(
        RunContext::Server,
        r#"
        local Players = game:GetService("Players")
        local Rig = Players:CreateHumanoidModelFromDescriptionAsync(Instance.new("HumanoidDescription"), Enum.HumanoidRigType.R6)
        local Names = {}
        for _, Child in Rig:GetChildren() do
            table.insert(Names, Child.Name)
        end
        table.sort(Names)
        print("rig", table.concat(Names, ","), Rig.PrimaryPart and Rig.PrimaryPart.Name, Rig.Humanoid.RigType.Name)
        local Neck = Rig.Torso.Neck
        print("neck", Neck.Part0.Name, Neck.Part1.Name, Rig.HumanoidRootPart.RootJoint.Part1.Name, #Rig.Torso:GetChildren())
        local Bare = Players:CreateHumanoidModelFromDescription(Instance.new("HumanoidDescription"), Enum.HumanoidRigType.R15)
        print("r15", #Bare:GetChildren(), Bare:FindFirstChildOfClass("Humanoid") ~= nil)
        print("cast", pcall(Players.CreateHumanoidModelFromDescription, Players, 5, Enum.HumanoidRigType.R6))
        local Before = time()
        local Ok, Message = pcall(game:GetService("InsertService").LoadAsset, game:GetService("InsertService"), 123)
        print("load", Ok, Message, time() > Before)
        local VersionBefore = time()
        local VersionOk, VersionMessage = pcall(game:GetService("InsertService").LoadAssetVersion, game:GetService("InsertService"), 456)
        print("version", VersionOk, VersionMessage, time() > VersionBefore)
        local AsyncBefore = time()
        local AsyncOk, AsyncMessage = pcall(game:GetService("AssetService").LoadAssetAsync, game:GetService("AssetService"), 789)
        print("async", AsyncOk, AsyncMessage, time() > AsyncBefore)
        local Player = Instance.new("Player")
        Player.Parent = Players
        print("character", select('#', Player:LoadCharacterAsync()), Player.Character)
        "#,
        0.2,
    );
    assert!(has(&lines, "print: rig Head,Humanoid,HumanoidRootPart,Left Arm,Left Leg,Right Arm,Right Leg,Torso HumanoidRootPart R6"), "{lines:?}");
    assert!(has(&lines, "print: neck Torso Head Torso 5"), "{lines:?}");
    assert!(has(&lines, "print: r15 1 true"), "{lines:?}");
    assert!(has_error(&lines, "cast false", "Unable to cast number to HumanoidDescription"), "{lines:?}");
    assert!(has(&lines, "print: load false InsertService:LoadAsset: asset 123 cannot be loaded offline true"), "{lines:?}");
    assert!(has(&lines, "print: version false InsertService:LoadAssetVersion: asset version 456 cannot be loaded offline true"), "{lines:?}");
    assert!(has(&lines, "print: async false AssetService:LoadAssetAsync: asset 789 cannot be loaded offline true"), "{lines:?}");
    assert!(has(&lines, "print: character 0 nil"), "{lines:?}");
}

#[test]
fn a_live_server_is_not_studio_and_its_players_carry_join_data() {
    let options = Options { mode: "play".into(), run_context: RunContext::Server, local_player: false, studio: false, ..Options::default() };
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    let set_join_data: LuaValue = lua.named_registry_value(crate::lua::stubs::SET_JOIN_DATA).expect("SetJoinData");
    let lines = run_with(
        &lua,
        &engine,
        r#"
        local RunService = game:GetService("RunService")
        local Players = game:GetService("Players")
        print("studio", RunService:IsStudio(), RunService:IsServer())
        local Host = Instance.new("Player")
        Host.UserId = 11
        local Plain = Instance.new("Player")
        local Given = { TeleportData = { Stage = "Stage1", MatchPlayers = { 11, 12 } }, Members = { 11, 12 } }
        SetJoinData(Host, Given)
        Given.TeleportData.Stage = "Changed"
        Host.Parent = Players
        Plain.Parent = Players
        local First, Second = Host:GetJoinData(), Host:GetJoinData()
        print("join", First.TeleportData.Stage, #First.TeleportData.MatchPlayers, First.Members[2], First ~= Second, next(Plain:GetJoinData()))
        print("bad", pcall(SetJoinData, Host, { Part = Instance.new("Part") }))
        print("not player", pcall(SetJoinData, workspace, {}))
        "#,
        0.1,
        &[("SetJoinData", set_join_data)],
    );
    assert!(has(&lines, "print: studio false true"), "{lines:?}");
    assert!(has(&lines, "print: join Stage1 2 12 true nil"), "{lines:?}");
    assert!(lines.iter().any(|line| line.starts_with("print: bad false") && line.contains("not Instance")), "{lines:?}");
    assert!(lines.iter().any(|line| line.starts_with("print: not player false") && line.contains("a Player expected")), "{lines:?}");
    let (studio_lua, studio_engine) = start(RunContext::Server);
    let studio = run_in(&studio_lua, &studio_engine, r#"print("studio", game:GetService("RunService"):IsStudio())"#, 0.0);
    assert!(has(&studio, "print: studio true"), "{studio:?}");
}

/// What follows a wait in a test script runs and is checked: the helper steps frames until the script ends (an assert after a
/// wait past the helper's clock used to pass unchecked, the call left waiting).
#[test]
#[should_panic(expected = "checked after the wait")]
fn a_test_script_is_checked_after_it_waits() {
    run(RunContext::Server, "task.wait(1) assert(false, 'checked after the wait')", 0.1);
}

/// A departing player's scene: a Player "Leaver" in Players (on a server, with the containers a joining player gets) and a
/// Model "Body" with a Humanoid and two anchored parts in the Workspace, the player's Character when `Character` is true; then
/// `Leave` (a statement) makes it leave. The script records the events a script sees, in the order they fire, and prints them
/// a frame on (`order ...`), what the player and the body hold in the leaving thread (`same thread ...`) and a frame on, and
/// when (time from the leave) the body and the player were destroyed six seconds on (`destroyed <body> <player>`, -1: not).
const LEAVING_SCENE: &str = r#"
local Players = game:GetService("Players")
local Order = {}
local Player = Instance.new("Player")
Player.Name = "Leaver"
Player.UserId = 5
Player.Parent = Players
local Body = Instance.new("Model")
Body.Name = "Body"
Instance.new("Humanoid").Parent = Body
for I = 1, 2 do
	local Part = Instance.new("Part")
	Part.Anchored = true
	Part.Position = Vector3.new(300, I * 4, 0)
	Part.Parent = Body
end
Body.Parent = workspace
if Character then
	Player.Character = Body
end
task.wait()
local function Mine(X)
	return X == Player or X == Body or X.Parent == Player
end
local function Note(Text)
	table.insert(Order, Text)
end
Players.DescendantRemoving:Connect(function(X) if Mine(X) then Note("Players.DescendantRemoving " .. X.Name) end end)
game.DescendantRemoving:Connect(function(X) if Mine(X) then Note("game.DescendantRemoving " .. X.Name) end end)
workspace.DescendantRemoving:Connect(function(X) if Mine(X) then Note("Workspace.DescendantRemoving " .. X.Name) end end)
workspace.ChildRemoved:Connect(function(X) if Mine(X) then Note("Workspace.ChildRemoved " .. X.Name) end end)
Player.CharacterRemoving:Connect(function(X) Note("CharacterRemoving " .. X.Name) end)
Body.AncestryChanged:Connect(function(X, Parent) if X == Body then Note("Body.AncestryChanged " .. tostring(Parent)) end end)
Player:GetPropertyChangedSignal("Character"):Connect(function() Note("Character " .. tostring(Player.Character)) end)
Player.Destroying:Connect(function() if #Order < 30 then Note("Leaver.Destroying") end end)
Players.PlayerRemoving:Connect(function(X) Note("PlayerRemoving " .. X.Name) end)
Players.ChildRemoved:Connect(function(X) if Mine(X) then Note("Players.ChildRemoved " .. X.Name) end end)
Player.AncestryChanged:Connect(function(X, Parent) if X == Player then Note("Leaver.AncestryChanged " .. tostring(Parent)) end end)
Player.Changed:Connect(function(Property) if Property == "Parent" then Note("Leaver.Changed Parent") end end)
local Gone = { Body = -1, Player = -1 }
local Left = time()
Leave
print("same thread", Player.Parent, Player.Character, Body.Parent)
Body.Destroying:Connect(function() Gone.Body = time() - Left end)
Player.Destroying:Connect(function() Gone.Player = time() - Left end)
task.wait()
print("order " .. table.concat(Order, ", "))
print("after", Player.Parent, Player.Character, Body.Parent)
task.wait(6)
print("destroyed", string.format("%.2f %.2f", Gone.Body, Gone.Player))
"#;

/// Runs [`LEAVING_SCENE`] with `leave` (the statement) on a fresh engine of `run_context`, the body the player's Character or
/// not, the place's Workspace.PlayerCharacterDestroyBehavior `behavior` (None: as the reflection's default leaves it), then
/// `after` (more statements, the scene's locals in reach).
#[inline(never)]
fn leaving(run_context: RunContext, leave: &str, character: bool, behavior: Option<&str>, after: &str) -> Vec<String> {
    let (lua, engine) = start(run_context);
    if let Some(behavior) = behavior {
        let workspace = engine.world().workspace;
        let name = engine.reflection.name_id("PlayerCharacterDestroyBehavior").unwrap();
        let item = engine.reflection.enum_item("PlayerCharacterDestroyBehavior", behavior).unwrap();
        super::instance::raw_set_silent(&mut engine.state.borrow_mut(), &engine.reflection, workspace, name, super::value::Value::EnumItem(item));
    }
    let source = format!("local Character = {character}\n{}\n{after}", LEAVING_SCENE.replace("\nLeave\n", &format!("\n{leave}\n")));
    run_in(&lua, &engine, &source, 7.0)
}

/// The scene of [`LEAVING_SCENE`] up to the leave, its player and body
#[inline(never)]
fn leaving_scene(character: bool) -> (Lua, Rc<Engine>, super::InstanceId, super::InstanceId) {
    let (lua, engine) = start(RunContext::Server);
    let source = format!("local Character = {character}\n{}", LEAVING_SCENE.split("\nlocal function Mine").next().unwrap());
    run_in(&lua, &engine, &source, 0.1);
    let players = crate::services::find_service(&engine, engine.world().game, "Players").unwrap();
    let workspace = engine.world().workspace;
    let (player, body) = {
        let state = engine.state.borrow();
        (state.dom.find_child(players, b"Leaver").unwrap(), state.dom.find_child(workspace, b"Body").unwrap())
    };
    (lua, engine, player, body)
}

/// What a server's script sees when a player leaves Players (Parent = nil here, Kick and Destroy below), as measured in 0.747
/// play sessions of one server and four clients (services/players.rs): Players' DescendantRemoving of the player, then the
/// player's CharacterRemoving, its character leaving the Workspace and Character becoming nil, then the rest of the player's
/// DescendantRemoving events, PlayerRemoving, and the player's own removal events, all done in the leaving thread; the
/// player's Parent locked; the character destroyed a second on and the Player five. The character used to stay in the
/// Workspace with Character still set, and PlayerRemoving fired last.
#[test]
fn a_leaving_players_character_leaves_with_it_and_is_destroyed_a_second_on() {
    let lines = leaving(
        RunContext::Server,
        "Player.Parent = nil",
        true,
        None,
        r#"print("locked", select(2, pcall(function() Player.Parent = Players end)))
        print("body", Body.Parent, #Body:GetChildren(), select(2, pcall(function() Body.Parent = workspace end)))"#,
    );
    let order = "print: order Players.DescendantRemoving Leaver, CharacterRemoving Body, Workspace.DescendantRemoving Body, \
        game.DescendantRemoving Body, Workspace.ChildRemoved Body, Body.AncestryChanged nil, Character nil, game.DescendantRemoving Leaver, \
        Players.DescendantRemoving StarterGear, game.DescendantRemoving StarterGear, Players.DescendantRemoving Backpack, \
        game.DescendantRemoving Backpack, PlayerRemoving Leaver, Players.ChildRemoved Leaver, Leaver.AncestryChanged nil, Leaver.Changed Parent";
    for wanted in [
        "print: same thread nil nil nil",
        order,
        "print: after nil nil nil",
        "print: destroyed 1.00 5.00",
        "print: locked The Parent property of Leaver is locked, current parent: NULL, new parent Players",
        "print: body nil 0 The Parent property of Body is locked, current parent: NULL, new parent Workspace",
    ] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

/// Kick and Destroy make the player leave the same way: Kick about a second on (the scene waits for it, then counts from the
/// leave), Destroy in the call, the player's Destroying first; a player whose Character is nil leaves with no
/// CharacterRemoving, its model left where it is.
#[test]
fn kick_and_destroy_take_the_character_with_the_player() {
    let order = "CharacterRemoving Body, Workspace.DescendantRemoving Body, game.DescendantRemoving Body, Workspace.ChildRemoved Body, \
        Body.AncestryChanged nil, Character nil, game.DescendantRemoving Leaver";
    let kicked = leaving(RunContext::Server, r#"Player:Kick("bye") while Player.Parent do task.wait() end Left = time()"#, true, None, "");
    for wanted in [
        "print: same thread nil nil nil".to_string(),
        format!("print: order Players.DescendantRemoving Leaver, {order}, Players.DescendantRemoving StarterGear"),
        "print: after nil nil nil".to_string(),
        "print: destroyed 1.00 5.00".to_string(),
    ] {
        assert!(kicked.iter().any(|line| line.starts_with(&wanted)), "{wanted}: {kicked:?}");
    }
    // (the player's children are destroyed too: their DescendantRemoving handlers find them parentless, so the scene leaves them out)
    let destroyed = leaving(RunContext::Server, "Player:Destroy()", true, None, "");
    for wanted in [
        "print: same thread nil nil nil".to_string(),
        format!("print: order Leaver.Destroying, Players.DescendantRemoving Leaver, {order}, PlayerRemoving Leaver, Players.ChildRemoved Leaver"),
        // (the player is destroyed already: only the body is, a second on)
        "print: destroyed 1.00 -1.00".to_string(),
    ] {
        assert!(destroyed.iter().any(|line| line.starts_with(&wanted)), "{wanted}: {destroyed:?}");
    }
    let plain = leaving(RunContext::Server, "Player.Parent = nil", false, None, r#"print("body", Body.Parent)"#);
    for wanted in ["print: same thread nil nil Workspace", "print: destroyed -1.00 5.00", "print: body Workspace"] {
        assert!(has(&plain, wanted), "{wanted}: {plain:?}");
    }
    assert!(plain.iter().any(|line| line.starts_with("print: order Players.DescendantRemoving Leaver, game.DescendantRemoving Leaver, ")), "{plain:?}");
}

/// A place whose Workspace.PlayerCharacterDestroyBehavior is Disabled keeps a departed player's character and the Player:
/// both only leave (the character unparented, the player's Parent locked), as measured; Enabled is the default's way.
#[test]
fn a_place_that_disables_the_destroy_keeps_departed_characters() {
    let lines = leaving(
        RunContext::Server,
        "Player.Parent = nil",
        true,
        Some("Disabled"),
        r#"print("locked", select(2, pcall(function() Player.Parent = Players end)))
        print("body", Body.Parent, #Body:GetChildren(), pcall(function() Body.Parent = workspace end))"#,
    );
    for wanted in [
        "print: same thread nil nil nil",
        "print: destroyed -1.00 -1.00",
        "print: locked The Parent property of Leaver is locked, current parent: NULL, new parent Players",
        "print: body nil 3 true",
    ] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
    let enabled = leaving(RunContext::Server, "Player.Parent = nil", true, Some("Enabled"), "");
    assert!(has(&enabled, "print: destroyed 1.00 5.00"), "{enabled:?}");
}

/// What a client sees of another player leaving (a harness's Player): the character's removal first, then CharacterRemoving
/// and Character's change, then (Destroy) the player's Destroying, then its own removal events with PlayerRemoving before
/// ChildRemoved; nothing is destroyed later (the server's destroys never reach a client), as measured.
#[test]
fn a_client_sees_the_character_leave_before_the_players_events() {
    let order = "Workspace.DescendantRemoving Body, game.DescendantRemoving Body, Workspace.ChildRemoved Body, Body.AncestryChanged nil, \
        CharacterRemoving Body, Character nil";
    let lines = leaving(RunContext::Client, "Player.Parent = nil", true, None, "");
    for wanted in [
        "print: same thread nil nil nil".to_string(),
        format!(
            "print: order {order}, Players.DescendantRemoving Leaver, game.DescendantRemoving Leaver, PlayerRemoving Leaver, Players.ChildRemoved Leaver, \
            Leaver.AncestryChanged nil, Leaver.Changed Parent"
        ),
        "print: destroyed -1.00 -1.00".to_string(),
    ] {
        assert!(has(&lines, &wanted), "{wanted}: {lines:?}");
    }
    let destroyed = leaving(RunContext::Client, "Player:Destroy()", true, None, "");
    let wanted = format!("print: order {order}, Leaver.Destroying, Players.DescendantRemoving Leaver, game.DescendantRemoving Leaver, PlayerRemoving Leaver");
    assert!(destroyed.iter().any(|line| line.starts_with(&wanted)), "{wanted}: {destroyed:?}");
}

/// The work clock: a script's leave pays for the character's removal what removing that model costs (its reparent's prices),
/// no more and no less; Kick's leave and the destroys after it are the engine's own step, so a kicked player's character costs
/// the run nothing.
#[test]
fn a_leaving_players_character_costs_what_its_removal_does() {
    let work = |engine: &Engine| engine.state.borrow().sched.work_total();
    let spent = |engine: &Rc<Engine>, id| {
        let before = work(engine);
        engine_lua::instance::set_parent(engine, id, None).unwrap();
        work(engine) - before
    };
    let (_lua, with, player, _) = leaving_scene(true);
    let leave_with = spent(&with, player);
    let (_lua, without, player, body) = leaving_scene(false);
    let (leave_alone, body_alone) = (spent(&without, player), spent(&without, body));
    assert!(body_alone > 0.0 && (leave_with - (leave_alone + body_alone)).abs() < 1e-12, "{leave_with} where {leave_alone} + {body_alone}");

    let kicked = |character: bool| {
        let (lua, engine, _, _) = leaving_scene(character);
        let before = work(&engine);
        run_in(&lua, &engine, r#"game:GetService("Players").Leaver:Kick()"#, 6.5);
        work(&engine) - before
    };
    let (with, without) = (kicked(true), kicked(false));
    assert!((with - without).abs() < 1e-12, "a kick's leave and destroys: {with} with the character, {without} without");
}

/// Kick leaves the player where it is for about a second, as a 0.747 server whose client ran in a process of its own (and a
/// client busy for four seconds, or reading its replication a second late, the same): its Parent, Character and place in
/// GetPlayers stay, a second Kick changes nothing, and it leaves at the network step of the frame after the first that starts
/// more than a second after the call, 62 frames at 60 Hz (services/players.rs kick). It used to leave at the next resumption
/// point.
#[test]
fn a_kicked_player_leaves_about_a_second_on() {
    let lines = run(
        RunContext::Server,
        r#"
local Players = game:GetService("Players")
local Player = Instance.new("Player")
Player.Name = "Leaver"
Player.Parent = Players
local Body = Instance.new("Model")
Body.Name = "Body"
Body.Parent = workspace
Player.Character = Body
local Frame = 0
game:GetService("RunService").Heartbeat:Connect(function() Frame += 1 end)
local Removing
Players.PlayerRemoving:Connect(function(X) if X == Player then Removing = Frame end end)
task.wait()
task.wait()
local F0, T0 = Frame, time()
Player:Kick("bye")
print("in the call", Player.Parent, Player.Character)
local During
while Player.Parent ~= nil do
	task.wait()
	if Frame - F0 == 10 then
		Player:Kick("again")
		During = string.format("%s %s %s %s", tostring(Player.Parent), tostring(Player.Character), tostring(Body.Parent), tostring(table.find(Players:GetPlayers(), Player) ~= nil))
	end
end
print("during", During)
print(string.format("left %d %.3f", Frame - F0, time() - T0))
task.wait()
print("removing", Removing - F0)
"#,
        3.0,
    );
    for wanted in ["print: in the call Players Body", "print: during Players Body Workspace true", "print: left 62 1.033", "print: removing 62"] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

/// A server's leave destroys the player's PlayerGui after the character's step and before the rest of the player's
/// DescendantRemoving events (measured: its Destroying, then its removal events), while the Backpack and StarterGear stay in
/// the departed player; a joining player's containers come in a 0.747 server's order, StarterGear, Backpack, PlayerGui. The
/// PlayerGui used to stay, and the containers came Backpack first.
#[test]
fn a_servers_leave_destroys_the_players_gui() {
    let lines = run(
        RunContext::Server,
        r#"
local Players = game:GetService("Players")
local Player = Instance.new("Player")
Player.Name = "Leaver"
Player.Parent = Players
local Names = {}
for _, Child in Player:GetChildren() do table.insert(Names, Child.Name) end
print("containers", table.concat(Names, " "))
local Gui, Pack, Gear = Player.PlayerGui, Player.Backpack, Player.StarterGear
local Order = {}
local function Tag(X) return if X == Player then "Leaver" elseif X == Gui then "PlayerGui" elseif X == Pack then "Backpack" elseif X == Gear then "StarterGear" else nil end
for _, Pair in { { "Leaver", Player }, { "Players", Players }, { "game", game } } do
	Pair[2].DescendantRemoving:Connect(function(X) if Tag(X) then table.insert(Order, Pair[1] .. ".DescendantRemoving " .. Tag(X)) end end)
end
Player.ChildRemoved:Connect(function(X) table.insert(Order, "Leaver.ChildRemoved " .. (Tag(X) or X.Name)) end)
Gui.Destroying:Connect(function() table.insert(Order, "PlayerGui.Destroying") end)
Gui.AncestryChanged:Connect(function(X, P) if X == Gui then table.insert(Order, "PlayerGui.AncestryChanged " .. tostring(P)) end end)
Players.PlayerRemoving:Connect(function() table.insert(Order, "PlayerRemoving") end)
task.wait()
Player.Parent = nil
task.wait()
print("order " .. table.concat(Order, ", "))
print("after", Gui.Parent, Pack.Parent, Gear.Parent)
"#,
        0.2,
    );
    for wanted in [
        "print: containers StarterGear Backpack PlayerGui",
        "print: order Players.DescendantRemoving Leaver, PlayerGui.Destroying, Leaver.DescendantRemoving PlayerGui, Players.DescendantRemoving PlayerGui, \
        game.DescendantRemoving PlayerGui, Leaver.ChildRemoved PlayerGui, PlayerGui.AncestryChanged nil, game.DescendantRemoving Leaver, \
        Players.DescendantRemoving StarterGear, game.DescendantRemoving StarterGear, Players.DescendantRemoving Backpack, game.DescendantRemoving Backpack, \
        PlayerRemoving",
        "print: after nil Leaver Leaver",
    ] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

/// A script's Character writes, as a 0.747 server and client measured them (services/players.rs set_character). `Prepare`:
/// statements run before the writes (the engine and `Player` in reach). Each case prints its events in the order they fired.
const CHARACTER_WRITES: &str = r#"
local Players = game:GetService("Players")
local Player = Instance.new("Player")
Player.Name = "P"
Player.Parent = Players
local Holder = Instance.new("Folder")
Holder.Name = "Holder"
Holder.Parent = game:GetService("ReplicatedStorage")
local function Model(Name, Parent) local M = Instance.new("Model") M.Name = Name M.Parent = Parent return M end
local Order = {}
local function Note(Text) table.insert(Order, Text) end
local Removed, Gone = {}, {}
local function Watch(M)
	M.AncestryChanged:Connect(function(X, P) if X == M then Note(M.Name .. ".AncestryChanged " .. tostring(P)) end end)
	M.Destroying:Connect(function() Gone[M.Name] = if Removed[M.Name] then time() - Removed[M.Name] else -1 end)
	return M
end
local M1 = Watch(Model("M1", workspace))
Player.Character = M1
task.wait()
Player.CharacterRemoving:Connect(function(X) Removed[X.Name] = time() Note("CharacterRemoving " .. X.Name) end)
Player.CharacterAdded:Connect(function(X) Note("CharacterAdded " .. X.Name) end)
Player.Changed:Connect(function(P) if P == "Character" then Note("Changed Character") end end)
Player:GetPropertyChangedSignal("Character"):Connect(function() Note("Character " .. tostring(Player.Character)) end)
workspace.ChildRemoved:Connect(function(X) Note("Workspace.ChildRemoved " .. X.Name) end)
local function Case(Label, Act)
	Act()
	local Thread = tostring(Player.Character)
	task.wait()
	print(Label .. " [" .. table.concat(Order, ", ") .. "] " .. Thread)
	table.clear(Order)
end
Case("replace", function() Player.Character = Watch(Model("M2", workspace)) print("same thread", M1.Parent) end)
Case("same", function() Player.Character = Player.Character end)
Case("elsewhere", function() Player.Character = Watch(Model("M3", Holder)) end)
Case("nil", function() Player.Character = nil end)
Case("parentless", function() Player.Character = Watch(Model("M4", nil)) end)
Case("from parentless", function() Player.Character = Watch(Model("M5", workspace)) end)
Case("destroyed", function() Player.Character:Destroy() Player.Character = Watch(Model("M6", workspace)) end)
task.wait(1.5)
local Times = {}
for _, Name in { "M1", "M2", "M3", "M4", "M5" } do table.insert(Times, Name .. " " .. (if Gone[Name] then string.format("%.2f", Gone[Name]) else "-")) end
print("destroyed " .. table.concat(Times, ", "))
"#;

/// A script's write of Character on a server takes the old character out (its removal events between CharacterRemoving and
/// CharacterAdded, done in the writing thread) and destroys it a second on, wherever it was, parentless too; CharacterAdded
/// fires before Changed; the same value fires nothing. Measured in 0.747 play sessions (a server and two clients, each case
/// twice). Character used to change alone, CharacterAdded after Changed.
#[test]
fn a_servers_character_write_takes_the_old_character_out() {
    let lines = run(RunContext::Server, CHARACTER_WRITES, 3.0);
    for wanted in [
        "print: same thread nil",
        "print: replace [CharacterRemoving M1, Workspace.ChildRemoved M1, M1.AncestryChanged nil, CharacterAdded M2, Changed Character, Character M2] M2",
        "print: same [] M2",
        "print: elsewhere [CharacterRemoving M2, Workspace.ChildRemoved M2, M2.AncestryChanged nil, CharacterAdded M3, Changed Character, Character M3] M3",
        "print: nil [CharacterRemoving M3, M3.AncestryChanged nil, Changed Character, Character nil] nil",
        "print: parentless [CharacterAdded M4, Changed Character, Character M4] M4",
        "print: from parentless [CharacterRemoving M4, CharacterAdded M5, Changed Character, Character M5] M5",
        "print: destroyed [Workspace.ChildRemoved M5, M5.AncestryChanged nil, CharacterRemoving M5, CharacterAdded M6, Changed Character, Character M6] M6",
        "print: destroyed M1 1.00, M2 1.00, M3 1.00, M4 1.00, M5 -1.00",
    ] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

/// A place whose Workspace.PlayerCharacterDestroyBehavior is Disabled keeps a replaced character, unparented (measured on a
/// 0.747 server, as for a departed player's).
#[test]
fn a_place_that_disables_the_destroy_keeps_replaced_characters() {
    let (lua, engine) = start(RunContext::Server);
    let workspace = engine.world().workspace;
    let name = engine.reflection.name_id("PlayerCharacterDestroyBehavior").unwrap();
    let item = engine.reflection.enum_item("PlayerCharacterDestroyBehavior", "Disabled").unwrap();
    super::instance::raw_set_silent(&mut engine.state.borrow_mut(), &engine.reflection, workspace, name, super::value::Value::EnumItem(item));
    let lines = run_in(&lua, &engine, &format!("{CHARACTER_WRITES}\nprint(\"kept\", M1.Parent, #Holder:GetChildren())"), 3.0);
    for wanted in ["print: destroyed M1 -, M2 -, M3 -, M4 -, M5 -1.00", "print: kept nil 0"] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

/// A client script's Character write fires CharacterRemoving, CharacterAdded and Changed as a server's does, and moves and
/// destroys nothing (measured in a 0.747 client, its own player and another's).
#[test]
fn a_clients_character_write_fires_its_events_and_moves_nothing() {
    let lines = run(RunContext::Client, CHARACTER_WRITES, 3.0);
    for wanted in [
        "print: same thread Workspace",
        "print: replace [CharacterRemoving M1, CharacterAdded M2, Changed Character, Character M2] M2",
        "print: same [] M2",
        "print: nil [CharacterRemoving M3, Changed Character, Character nil] nil",
        "print: destroyed M1 -, M2 -, M3 -, M4 -, M5 -1.00",
    ] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
}

/// The work clock: a server script's Character write pays the old character's removal what that reparent costs (as a leave
/// does) on top of the write, and nothing for the old character's destroy a second on (the engine's own step).
#[test]
fn a_character_write_costs_the_old_characters_removal() {
    let scene = |held: bool| {
        let (lua, engine) = start(RunContext::Server);
        let source = format!(
            r#"
local Player = Instance.new("Player")
Player.Name = "P"
Player.Parent = game:GetService("Players")
local function Model(Name) local M = Instance.new("Model") M.Name = Name local Part = Instance.new("Part") Part.Anchored = true Part.Parent = M M.Parent = workspace return M end
local Old, New = Model("Old"), Model("New")
if {held} then Player.Character = Old end
"#
        );
        run_in(&lua, &engine, &source, 0.1);
        let players = crate::services::find_service(&engine, engine.world().game, "Players").unwrap();
        let workspace = engine.world().workspace;
        let (player, old, new) = {
            let state = engine.state.borrow();
            (state.dom.find_child(players, b"P").unwrap(), state.dom.find_child(workspace, b"Old").unwrap(), state.dom.find_child(workspace, b"New").unwrap())
        };
        (lua, engine, player, old, new)
    };
    let work = |engine: &Engine| engine.state.borrow().sched.work_total();
    let write = |engine: &Rc<Engine>, player, new| {
        let before = work(engine);
        let character = engine.reflection.name_id("Character").unwrap();
        engine_lua::instance::script_set_property(engine, player, character, super::value::Value::Instance(new));
        work(engine) - before
    };
    let (with_lua, with, player, _, new) = scene(true);
    let replaced = write(&with, player, new);
    let (without_lua, without, player, old, new) = scene(false);
    let set = write(&without, player, new);
    let before = work(&without);
    engine_lua::instance::set_parent(&without, old, None).unwrap();
    let removal = work(&without) - before;
    assert!(removal > 0.0 && (replaced - (set + removal)).abs() < 1e-12, "{replaced} where {set} + {removal}");
    let idle = |lua: &Lua, engine: &Rc<Engine>| {
        let before = work(engine);
        let now = engine.state.borrow().sched.now;
        scheduler::run_until(lua, engine, now + 1.5).unwrap();
        work(engine) - before
    };
    let (after_with, after_without) = (idle(&with_lua, &with), idle(&without_lua, &without));
    assert!((after_with - after_without).abs() < 1e-12, "the destroy: {after_with} against {after_without}");
}

/// A server's StarterGui answers a client's core GUI calls with a warning (measured on a 0.747 server, service_answers.luau): the
/// arguments cast as a client casts them, then "StarterGui:SetCoreGuiEnabled must be called from a local script." and nothing
/// done, GetCoreGuiEnabled false after its warning, SetCore a warning and nothing. ree answered true and reported them missing.
#[test]
fn a_servers_core_gui_calls_warn() {
    let lines = run(
        RunContext::Server,
        r#"
        local SG = game:GetService("StarterGui")
        SG:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, false)
        print(SG:GetCoreGuiEnabled(Enum.CoreGuiType.Chat), pcall(SG.SetCoreGuiEnabled, SG, 99, false))
        SG:SetCore("Nope", 1)
        "#,
        0.1,
    );
    let wanted: Vec<&str> = lines.iter().filter(|line| line.starts_with("print: ") || line.starts_with("warn: ")).map(String::as_str).collect();
    assert_eq!(
        wanted,
        [
            "warn: StarterGui:SetCoreGuiEnabled must be called from a local script.",
            "warn: StarterGui:GetCoreGuiEnabled must be called from a local script.",
            "print: false false Unable to cast double to CoreGuiType",
            "warn: StarterGui:SetCore must be called from a local script.",
        ],
        "{lines:?}"
    );
}
