//! The profiling suite (profiler/): each profiler checked against what the VM and the engine count themselves: the
//! instruction profile against the VM's own work counters, the booked work against the work clock, the allocation
//! profile against the VM's heap count, the heap snapshot against what a script holds, the timeline against the frames.

use std::rc::Rc;

use mlua::prelude::*;

use crate::{
    Engine, Options,
    lua::{self as engine_lua, globals::CallOutcome},
    profiler::{self, Config, Kinds, memory, vm},
    scheduler,
};

/// Profiles share process-wide switches with luau_profile's (the caches' switch, the ticker): one test at a time
fn lock() -> std::sync::MutexGuard<'static, ()> {
    crate::luau_profile::TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn start() -> (Lua, Rc<Engine>) {
    let engine = Engine::new(Options::default()).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    // (interpreted, as `ree run` and `ree record` run modules: every instruction is the interpreter's)
    engine_lua::modules::interpret_all(&lua, &engine);
    (lua, engine)
}

/// Runs `source` as a script (chunk `=name`) on a scheduler thread; its output lines
fn run(lua: &Lua, engine: &Rc<Engine>, name: &str, source: &str) -> Vec<String> {
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        crate::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(lua, engine, script).expect("environment");
    let function = lua.load(source).set_name(format!("={name}")).set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(lua, engine, function, LuaMultiValue::new()).expect("call");
    if let CallOutcome::Failed(message, _) = call.outcome().unwrap() {
        panic!("test script failed: {message}");
    }
    engine.state.borrow().output.entries.iter().map(|entry| entry.message.clone()).collect()
}

fn main_state(lua: &Lua) -> *mut mlua::ffi::lua_State {
    unsafe { mlua::ffi::lua_mainthread(lua.current_thread().state()) }
}

fn ops(lua: &Lua) -> (u64, u64) {
    let counters = unsafe { &*crate::cost::lua_workcounters(main_state(lua)) };
    (counters.ops.iter().sum(), counters.hostops.iter().sum())
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("ree-profile-{name}-{}-{stamp}", std::process::id()))
}

/// Every instruction a script dispatches is counted on its own instruction, exactly what the VM's counters count;
/// a loop's lines carry its iterations, calls are counted per callee, and the call tree's totals hold the callees'.
#[test]
fn the_instruction_profile_counts_every_instruction_the_vm_counts() {
    let _lock = lock();
    let lua = Lua::new();
    // (interpreted: a natively compiled chunk dispatches no instruction)
    lua.enable_jit(false);
    let state = main_state(&lua);
    let source = "local function leaf(x)\n\treturn x * 2\nend\nlocal function middle(n)\n\tlocal total = 0\n\tfor i = 1, n do\n\t\ttotal += leaf(i)\n\tend\n\treturn total\nend\nlocal sum = 0\nfor round = 1, 10 do\n\tsum += middle(100)\nend\nresult = sum\n";
    let function = lua.load(source).set_name("=probe").into_function().unwrap();
    let before = ops(&lua);
    assert!(vm::start(state, true, None, None));
    function.call::<()>(()).unwrap();
    vm::stop(state);
    let after = ops(&lua);
    let results = vm::results(state);
    vm::free(state);
    assert_eq!(lua.globals().get::<f64>("result").unwrap(), 10.0 * 2.0 * 5050.0);
    let counted: u64 = results.functions.iter().filter(|f| !f.exempt()).flat_map(|f| f.counts.iter()).sum();
    assert_eq!(counted, after.0 - before.0, "the profile counts what the VM's counters count");
    let by_name = |name: &str| results.functions.iter().position(|f| f.name == name).unwrap_or_else(|| panic!("{name} in {:?}", results.functions.iter().map(|f| &f.name).collect::<Vec<_>>()));
    let (leaf, middle) = (by_name("leaf"), by_name("middle"));
    let line_count = |function: usize, line: i32| -> u64 { results.functions[function].counts.iter().zip(&results.functions[function].lines).filter(|(_, l)| **l == line).map(|(c, _)| *c).sum() };
    // leaf's line 2 runs its MUL and RETURN once a call: 1,000 calls
    assert_eq!(line_count(leaf, 2), 2000, "leaf's body");
    // middle's loop body: the call to leaf (GETUPVAL or MOVE, MOVE, CALL, ADD) and its FORNLOOP 100 times a call
    assert!(line_count(middle, 7) >= 4000, "middle's loop body: {}", line_count(middle, 7));
    let calls = |function: usize| -> u64 { results.nodes.iter().filter(|node| node.function == function as i32).map(|node| node.calls).sum() };
    assert_eq!(calls(leaf), 1000);
    assert_eq!(calls(middle), 10);
    // middle's nodes hold leaf's as children: its subtree counts leaf's instructions too
    let leaf_insns: u64 = results.functions[leaf].counts.iter().sum();
    let middle_node = results.nodes.iter().position(|node| node.function == middle as i32).unwrap();
    let leaf_under_middle: u64 = results.nodes.iter().filter(|node| node.parent == middle_node as i32 && node.function == leaf as i32).map(|node| node.insns).sum();
    assert_eq!(leaf_under_middle, leaf_insns, "every leaf call is under middle");
    // the call site of leaf is middle's line 7
    let site = results.nodes.iter().find(|node| node.function == leaf as i32).unwrap().callpc;
    assert_eq!(results.functions[middle].lines[site as usize], 7);
}

/// What the profile books to the instructions adds up to the work clock's own total for the same span: the VM's
/// counters priced as cost::luau prices them, the engine's charges (Instance API, datatypes) and what was charged
/// between instructions; and a profiled run's work clock is the unprofiled run's, to the bit.
#[test]
fn booked_work_adds_up_to_the_work_clock_and_leaves_it_unchanged() {
    let _lock = lock();
    let script = "local parts = {}\nfor i = 1, 200 do\n\tlocal part = Instance.new(\"Part\")\n\tpart.Name = \"P\" .. i\n\tpart.Transparency = i / 200\n\tparts[i] = part\nend\nlocal total = 0\nfor _, part in parts do\n\ttotal += part.Transparency + part.CFrame.Position.X\nend\nprint(os.clock() > 0, total)\n";
    let measure = |profiled: bool| -> (f64, f64, Vec<String>) {
        let (lua, engine) = start();
        scheduler::step(&lua, &engine, Some(1.0 / 60.0)).unwrap();
        let state = main_state(&lua);
        let (work, drained) = {
            let state = engine.state.borrow();
            (state.sched.work.clone(), state.sched.work_drained.clone())
        };
        let before = engine.state.borrow().sched.work_total();
        if profiled {
            let prices = vm::prices().expect("the work clock's model is a weighted sum");
            assert!(vm::start(state, true, Some(&prices), Some((work.as_ptr() as *const f64, drained.as_ptr() as *const f64))));
        }
        let output = run(&lua, &engine, "work", script);
        scheduler::step(&lua, &engine, Some(1.0 / 60.0)).unwrap();
        let after = engine.state.borrow().sched.work_total();
        let mut booked = 0.0;
        if profiled {
            vm::stop(state);
            let results = vm::results(state);
            vm::free(state);
            booked = results.nodes.iter().map(|node| node.work).sum::<f64>() + results.engine_work;
        }
        (after - before, booked, output)
    };
    let (plain, _, plain_output) = measure(false);
    let (profiled, booked, profiled_output) = measure(true);
    assert_eq!(plain.to_bits(), profiled.to_bits(), "profiling moved the work clock: {plain} -> {profiled}");
    assert_eq!(plain_output, profiled_output);
    assert!(plain > 50e-6, "the script did work: {plain}");
    assert!((booked - profiled).abs() <= 1e-9 * profiled.max(1e-9) + 1e-12, "booked {booked} of the work clock's {profiled}");
}

/// The inline caches are read as the interpreter reads them: a table key read from one table hits its slot after its
/// first miss; a key the table lacks is looked up every time (a miss); a key its metatable's __index gives takes the
/// slow path; a global hits its slot.
#[test]
fn inline_caches_tell_hits_from_misses() {
    let _lock = lock();
    let lua = Lua::new();
    lua.enable_jit(false);
    let state = main_state(&lua);
    let source = "local same = {a = 1, b = 2, c = 3}\nlocal x = 0\nfor i = 1, 100 do\n\tx += same.c\nend\nlocal empty = {}\nfor i = 1, 100 do\n\tx += empty.c or 0\nend\nlocal inherited = setmetatable({}, {__index = {c = 1}})\nfor i = 1, 100 do\n\tx += inherited.c\nend\nG = 0\nfor i = 1, 100 do\n\tG += 1\nend\nresult = x\n";
    let function = lua.load(source).set_name("=caches").into_function().unwrap();
    assert!(vm::start(state, true, None, None));
    function.call::<()>(()).unwrap();
    vm::stop(state);
    let results = vm::results(state);
    vm::free(state);
    let main = results.functions.iter().find(|f| f.source == "caches" && f.line == 0).unwrap_or_else(|| &results.functions[0]);
    let outcomes = |line: i32, op: &str| -> [u64; 4] {
        let mut total = [0u64; 4];
        for pc in 0..main.code.len() {
            if main.lines[pc] == line && vm::opcode_name((main.code[pc] & 0xff) as u8) == op && !main.caches.is_empty() {
                for slot in 0..4 {
                    total[slot] += main.caches[pc * vm::CACHE_OUTCOMES + slot];
                }
            }
        }
        total
    };
    let same = outcomes(4, "GETTABLEKS");
    assert_eq!(same.iter().sum::<u64>(), 100);
    assert!(same[0] >= 99, "one shape: hits after the first lookup {same:?}");
    let absent = outcomes(8, "GETTABLEKS");
    assert_eq!(absent, [0, 100, 0, 0], "a key the table lacks: looked up every time");
    let inherited = outcomes(12, "GETTABLEKS");
    assert_eq!(inherited, [0, 0, 100, 0], "a key its metatable gives: the metatable's path");
    let globals = outcomes(16, "GETGLOBAL");
    assert!(globals[0] >= 99 && globals.iter().sum::<u64>() == 100, "{globals:?}");
}

/// Allocations are typed exactly (every one), and their stacks name the allocating line; an Instance's userdata
/// names its class; table storage and strings are told apart.
#[test]
fn allocations_are_typed_and_found_by_their_stack() {
    let _lock = lock();
    let (lua, engine) = start();
    let config = Config::new(Kinds::ALLOC, temp_dir("alloc"));
    let session = profiler::Session::begin(config.clone());
    crate::luau_profile::install(&lua);
    run(&lua, &engine, "alloc", "local kept = {}\nlocal function tables()\n\tfor i = 1, 500 do\n\t\tkept[#kept + 1] = {i}\n\tend\nend\nlocal function strings()\n\tfor i = 1, 300 do\n\t\tkept[#kept + 1] = string.rep(\"x\", 100 + i)\n\tend\nend\nlocal function parts()\n\tfor i = 1, 50 do\n\t\tkept[#kept + 1] = Instance.new(\"Part\")\n\tend\nend\ntables()\nstrings()\nparts()\n");
    profiler::collect(&lua);
    let summary = session.finish(profiler::Run { command: "test".into(), status: "ok".into(), ..Default::default() }).unwrap();
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config.out.join("profile.json")).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&config.out);
    let by_type: std::collections::HashMap<String, f64> = json["alloc"]["by_type"].as_array().unwrap().iter().map(|row| (row["type"].as_str().unwrap().to_string(), row["count"].as_f64().unwrap())).collect();
    assert!(by_type["table"] >= 500.0, "{by_type:?}");
    assert!(by_type["string"] >= 300.0, "{by_type:?}");
    assert!(by_type.get("userdata: Instance Part").copied().unwrap_or(0.0) >= 50.0, "an Instance's class: {by_type:?}\n{summary}");
    assert!(by_type.contains_key("table storage"), "{by_type:?}");
    // the stacks: the table constructor's line in `tables`, string.rep's caller in `strings`
    let stacks = json["alloc"]["by_stack"].as_array().unwrap();
    let functions = json["functions"].as_array().unwrap();
    let id_of = |name: &str| functions.iter().find(|f| f["name"] == name).and_then(|f| f["id"].as_i64()).unwrap_or_else(|| panic!("{name}"));
    let at = |name: &str, line: i64, ty: &str| -> f64 {
        stacks.iter().filter(|row| row["type"] == ty && row["frames"].as_array().unwrap().iter().any(|frame| frame[0].as_i64() == Some(id_of(name)) && frame[1].as_i64() == Some(line))).map(|row| row["count"].as_f64().unwrap()).sum()
    };
    assert_eq!(at("tables", 4, "table"), 500.0);
    assert_eq!(at("strings", 9, "string"), 300.0);
}

/// A heap snapshot sees what a script holds: by type, the biggest by what they keep alive with the path that holds
/// them, the run's own objects by the stack that made them (a freed object leaves), and garbage not collected yet.
#[test]
fn a_heap_snapshot_finds_what_holds_memory() {
    let _lock = lock();
    let (lua, engine) = start();
    let config = Config::new(Kinds::HEAP.with(Kinds::ALLOC), temp_dir("heap"));
    let session = profiler::Session::begin(config.clone());
    crate::luau_profile::install(&lua);
    run(&lua, &engine, "heap", "local function fill()\n\tlocal big = {}\n\tfor i = 1, 4000 do\n\t\tbig[i] = {i, tostring(i)}\n\tend\n\treturn big\nend\n_G.HeldCache = fill()\nlocal function garbage()\n\tfor i = 1, 2000 do\n\t\tlocal _ = {i}\n\tend\nend\ngarbage()\n");
    lua.gc_collect().unwrap();
    profiler::collect(&lua);
    session.finish(profiler::Run { command: "test".into(), status: "ok".into(), ..Default::default() }).unwrap();
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config.out.join("profile.json")).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&config.out);
    let snapshots = json["heap"]["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.first().unwrap()["label"], "start");
    let end = snapshots.last().unwrap();
    assert_eq!(end["label"], "end");
    // the cache: 4,001 tables and 4,000 strings kept alive by the table the global names (the threads and environments
    // that hold it keep it too: they come before it)
    let biggest = end["biggest"].as_array().unwrap();
    let cache = biggest.iter().find(|held| held["path"].as_str().unwrap().ends_with("HeldCache")).unwrap_or_else(|| panic!("{biggest:#?}"));
    assert!(cache["retained"].as_u64().unwrap() > 4000 * 60, "{cache}");
    assert_eq!(cache["type"], "table");
    // the run's live objects by stack: fill's tables and strings are there; garbage's are not (collected, unmarked)
    let functions = json["functions"].as_array().unwrap();
    let id_of = |name: &str| functions.iter().find(|f| f["name"] == name).and_then(|f| f["id"].as_i64()).unwrap_or(-99);
    let live = |name: &str| -> f64 {
        end["by_stack"].as_array().unwrap().iter().filter(|row| row["frames"].as_array().unwrap().iter().any(|frame| frame[0].as_i64() == Some(id_of(name)))).map(|row| row["count"].as_f64().unwrap()).sum()
    };
    assert!(live("fill") >= 8000.0, "fill's 4,000 tables and 4,000 strings live: {}", live("fill"));
    assert_eq!(live("garbage"), 0.0, "garbage's tables were freed");
}

/// The collector's steps, cycles and bytes freed come from the VM's interrupt at each step.
#[test]
fn the_collectors_work_is_counted() {
    let _lock = lock();
    let (lua, engine) = start();
    let config = Config::new(Kinds::HEAP, temp_dir("gc"));
    let session = profiler::Session::begin(config.clone());
    crate::luau_profile::install(&lua);
    run(&lua, &engine, "churn", "for i = 1, 200000 do\n\tlocal _ = {i, i + 1, i + 2}\nend\n");
    profiler::collect(&lua);
    session.finish(profiler::Run { command: "test".into(), status: "ok".into(), ..Default::default() }).unwrap();
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config.out.join("profile.json")).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&config.out);
    let gc = &json["heap"]["gc"];
    assert!(gc["steps"].as_u64().unwrap() > 10, "{gc}");
    assert!(gc["cycles"].as_u64().unwrap() >= 1, "{gc}");
    assert!(gc["freed"].as_u64().unwrap() > 1_000_000, "{gc}");
}

/// The timeline has a row a frame, with the frame's work, the heap, live instances, connections and threads.
#[test]
fn the_timeline_has_a_row_a_frame() {
    let _lock = lock();
    // (a playing client: one Heartbeat a frame; edit mode beats at 240 Hz)
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    engine_lua::modules::interpret_all(&lua, &engine);
    let config = Config::new(Kinds::TIMELINE.with(Kinds::WORK), temp_dir("timeline"));
    let session = profiler::Session::begin(config.clone());
    crate::luau_profile::install(&lua);
    run(&lua, &engine, "frames", "local RunService = game:GetService(\"RunService\")\nlocal made = {}\nRunService.Heartbeat:Connect(function()\n\tmade[#made + 1] = Instance.new(\"Folder\")\n\ttask.spawn(function() task.wait(10) end)\nend)\n");
    for _ in 0..10 {
        scheduler::step(&lua, &engine, Some(1.0 / 60.0)).unwrap();
    }
    profiler::collect(&lua);
    session.finish(profiler::Run { command: "test".into(), status: "ok".into(), frames: 10, seconds: 10.0 / 60.0, fps: 60.0, ..Default::default() }).unwrap();
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config.out.join("profile.json")).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&config.out);
    let timeline = &json["timeline"];
    let columns: Vec<&str> = timeline["columns"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    let rows = timeline["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 10);
    let column = |name: &str| columns.iter().position(|c| *c == name).unwrap();
    let (first, last) = (&rows[0], &rows[9]);
    assert_eq!(last[column("instances")].as_u64().unwrap() - first[column("instances")].as_u64().unwrap(), 9, "a Folder a frame");
    assert!(last[column("threads")].as_i64().unwrap() - first[column("threads")].as_i64().unwrap() >= 9, "a waiting thread a frame");
    assert!(rows.iter().all(|row| row[column("work_ms")].as_f64().unwrap() > 0.0), "each frame's handler did work");
}

/// ree's own caches are counted while a profile asks: a property read twice hits the member dispatch the second time,
/// and an Instance pushed again hits its userdata.
#[test]
fn ree_caches_count_hits_and_misses() {
    let _lock = lock();
    let (lua, engine) = start();
    let config = Config::new(Kinds::CACHES, temp_dir("caches"));
    let session = profiler::Session::begin(config.clone());
    crate::luau_profile::install(&lua);
    run(&lua, &engine, "caches", "local part = Instance.new(\"Part\")\nlocal folder = Instance.new(\"Folder\")\npart.Parent = folder\nfor i = 1, 100 do\n\tlocal _ = part.Transparency\n\tlocal _ = part.Parent\nend\n");
    profiler::collect(&lua);
    session.finish(profiler::Run { command: "test".into(), status: "ok".into(), ..Default::default() }).unwrap();
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config.out.join("profile.json")).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&config.out);
    let cache = |name: &str| json["ree_caches"].as_array().unwrap().iter().find(|row| row["name"] == name).cloned().unwrap();
    let members = cache("member dispatch");
    assert!(members["hits"].as_u64().unwrap() >= 198, "{members}");
    let wrappers = cache("instance userdata");
    assert!(wrappers["hits"].as_u64().unwrap() >= 99, "the folder's userdata found again: {wrappers}");
    assert!(json["luau_caches"]["by_kind"].as_array().is_some(), "Luau's caches come with the stepping");
}

/// The command line: kinds parse with their aliases; `--profile` alone and `--profile boundary` alone keep their own
/// forms (the phases, the boundary profiler) and give no suite profile; a kinds value takes `--profile-out`.
#[test]
fn profile_options_keep_the_old_forms() {
    let args = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    assert_eq!(Kinds::parse("all").unwrap(), Kinds::ALL);
    assert!(Kinds::parse("instructions").unwrap().has(Kinds::WORK));
    assert!(Kinds::parse("memory").unwrap().has(Kinds::HEAP));
    assert!(Kinds::parse("nonsense").unwrap_err().contains("nonsense"));
    let none = || std::path::PathBuf::from("default");
    assert!(Config::from_args(&args(&["--harness", "x.luau", "--profile"]), none).unwrap().is_none());
    assert!(Config::from_args(&args(&["--profile", "--seconds", "2"]), none).unwrap().is_none());
    assert!(Config::from_args(&args(&["--profile", "boundary", "--profile-out", "b.json"]), none).unwrap().is_none());
    assert!(profiler::phases_requested(&args(&["--profile", "--seconds", "2"])));
    assert!(!profiler::phases_requested(&args(&["--profile", "work"])));
    let config = Config::from_args(&args(&["--profile", "work,alloc", "--profile-out", "dir", "--profile-top", "7"]), none).unwrap().unwrap();
    assert!(config.kinds.has(Kinds::WORK) && config.kinds.has(Kinds::ALLOC) && !config.kinds.has(Kinds::HEAP));
    assert_eq!((config.out, config.top), (std::path::PathBuf::from("dir"), 7));
    assert_eq!(profiler::profile_arguments(&args(&["--profile", "work", "--x"]), 0), 2);
    assert_eq!(profiler::profile_arguments(&args(&["--profile", "--x"]), 0), 1);
}

/// A run writes every file (summary, JSON, folded stacks, speedscope, the page), the page holds the profile, and a diff
/// of a profile with itself finds nothing moved while one with more work finds the function that did it.
#[test]
fn a_profile_writes_its_files_and_diffs() {
    let _lock = lock();
    let profile = |name: &str, rounds: u32| -> std::path::PathBuf {
        let (lua, engine) = start();
        let config = Config { print: false, ..Config::new(Kinds::ALL, temp_dir(name)) };
        let session = profiler::Session::begin(config.clone());
        crate::luau_profile::install(&lua);
        crate::luau_profile::set_phase("frames");
        // (a table's function: -O2 inlines a small local function into its caller)
        run(&lua, &engine, "work", &format!("local M = {{}}\nfunction M.hot(n)\n\tlocal t = {{}}\n\tfor i = 1, n do\n\t\tt[i] = i * i\n\tend\n\treturn t\nend\nfor round = 1, {rounds} do\n\tM.hot(200)\nend\n"));
        scheduler::step(&lua, &engine, Some(1.0 / 60.0)).unwrap();
        profiler::collect(&lua);
        let summary = session.finish(profiler::Run { command: "test".into(), status: "ok".into(), frames: 1, seconds: 1.0 / 60.0, fps: 60.0, ..Default::default() }).unwrap();
        assert!(summary.contains("top functions by self work") && summary.contains("hot"), "{summary}");
        config.out
    };
    let (before, after) = (profile("before", 5), profile("after", 10));
    for file in ["summary.txt", "profile.json", "report.html", "work.folded", "instructions.folded", "alloc.folded", "profile.speedscope.json"] {
        assert!(before.join(file).is_file(), "{file}");
    }
    let page = std::fs::read_to_string(before.join("report.html")).unwrap();
    assert!(page.contains("\"version\":1") && !page.contains("/*PROFILE_DATA*/"), "the page carries the profile");
    let speedscope: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(before.join("profile.speedscope.json")).unwrap()).unwrap();
    assert!(speedscope["profiles"].as_array().unwrap().len() >= 3);
    let load = |dir: &std::path::Path| -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(dir.join("profile.json")).unwrap()).unwrap() };
    let (same, _) = profiler::diff::compare(&load(&before), &load(&before), 10);
    assert!(!same.contains("functions that moved most"), "{same}");
    let (moved, json) = profiler::diff::compare(&load(&before), &load(&after), 10);
    assert!(moved.contains("hot"), "{moved}");
    let hot = json["functions"].as_array().unwrap().iter().find(|row| row["function"].as_str().unwrap().starts_with("M.hot") || row["function"].as_str().unwrap().starts_with("hot")).unwrap_or_else(|| panic!("{moved}"));
    assert!(hot["after"]["calls"].as_f64().unwrap() == 2.0 * hot["before"]["calls"].as_f64().unwrap(), "{hot}");
    let _ = std::fs::remove_dir_all(&before);
    let _ = std::fs::remove_dir_all(&after);
}

/// The memory module's own figures: a stack is interned once, a type name once.
#[test]
fn stacks_and_types_intern_once() {
    let mut stacks = memory::Stacks::default();
    let a = stacks.intern(&[(1, 10), (2, 20)]);
    let b = stacks.intern(&[(1, 10), (2, 20)]);
    let c = stacks.intern(&[(1, 11)]);
    assert_eq!((a, b, c), (0, 0, 1));
    let mut types = memory::Types::default();
    assert_eq!((types.id("table"), types.id("string"), types.id("table")), (0, 1, 0));
    assert_eq!(memory::group("table storage"), "table");
    assert_eq!(memory::group("userdata: Instance Part"), "userdata");
}
