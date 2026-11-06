//! The allocation profile (`RUR_LUAU_ALLOC_PROFILE`): bytes and counts by Luau stack, checked against
//! what the VM itself reports allocating (`collectgarbage("count")` with the collector stopped).

use std::collections::HashMap;

use mlua::prelude::*;

use crate::luau_profile;

/// Each function allocates a known amount; line numbers matter (`lines` allocates on 10 and 11).
const PROBE: &str = "collectgarbage('stop')\n\
local keep = {}\n\
local function measure(f) local before = collectgarbage('count') f() return (collectgarbage('count') - before) * 1024 end\n\
local function tables() for i = 1, 3000 do keep[1] = {i, i, i, i} end end\n\
local function growth() local t = {} for i = 1, 20000 do t[i] = i end end\n\
local function strings() for i = 1, 400 do keep[1] = string.rep('x', 500 + i) end end\n\
local function depth(n) if n > 0 then keep[1] = {n} depth(n - 1) end end\n\
local function lines()\n\
  for i = 1, 1000 do\n\
    keep[1] = {i}\n\
    keep[2] = {i, i, i, i, i, i, i, i}\n\
  end\n\
end\n\
measured = {tables = measure(tables), growth = measure(growth), strings = measure(strings), depth = measure(function() depth(5000) end), lines = measure(lines)}\n";

struct Profile {
    measured: HashMap<String, f64>,
    bytes: Vec<(String, u64)>,
    counts: Vec<(String, u64)>,
}

fn read(path: &std::path::Path) -> Vec<(String, u64)> {
    let text = std::fs::read_to_string(path).expect("profile file");
    text.lines()
        .map(|line| {
            let (stack, weight) = line.rsplit_once(' ').expect("folded line");
            (stack.to_string(), weight.parse().expect("weight"))
        })
        .collect()
}

/// Runs [`PROBE`] under an allocation profile taking one sample per `step` bytes.
fn profile(step: u64) -> Profile {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("ree-alloc-profile-{}-{step}-{stamp}", std::process::id()));
    let lua = Lua::new();
    luau_profile::begin_allocations(path.clone(), step);
    let guard = luau_profile::ProfileGuard;
    luau_profile::hook_allocations(&lua);
    lua.load(PROBE).set_name("=probe").exec().expect("probe");
    drop(guard);
    let measured = lua.globals().get::<LuaTable>("measured").expect("measured").pairs::<String, f64>().map(|pair| pair.unwrap()).collect();
    let counts_path = std::path::PathBuf::from(format!("{}.counts", path.display()));
    let profile = Profile { measured, bytes: read(&path), counts: read(&counts_path) };
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&counts_path);
    profile
}

/// What the profile attributes to stacks passing through the function `name` of the probe.
fn within(rows: &[(String, u64)], name: &str) -> f64 {
    let frame = format!(";{name} probe:");
    rows.iter().filter(|(stack, _)| stack.contains(&frame)).map(|row| row.1 as f64).sum()
}

#[test]
fn every_allocation_is_attributed_to_its_stack() {
    let exact = profile(1);
    for name in ["tables", "growth", "strings", "depth", "lines"] {
        let (attributed, measured) = (within(&exact.bytes, name), exact.measured[name]);
        assert!(measured > 10_000.0, "{name} allocates {measured}");
        // (bytes allocated, not net: string.rep's buffer shrinks to the string, which the net count nets off, 1.2 %)
        assert!((attributed - measured).abs() <= measured * 0.02, "{name}: {attributed} attributed, {measured} allocated");
    }
    // one table and one array per constructor
    assert_eq!(within(&exact.counts, "tables"), 6000.0, "{:?}", exact.counts);
    // a native function's allocations end the stack in it, under the script that called it
    let rep: u64 = exact.bytes.iter().filter(|(stack, _)| stack.contains(";strings probe:6 @6;rep [C]")).map(|row| row.1).sum();
    assert!(rep as f64 > exact.measured["strings"] * 0.99, "{rep} of {:?}", exact.bytes);
    // lines name the allocating line of their function
    let at = |line: u32| -> u64 { exact.bytes.iter().filter(|(stack, _)| stack.contains(&format!(";lines probe:8 @{line}"))).map(|row| row.1).sum() };
    assert!(at(10) > 0 && at(11) > at(10) * 2, "{} {}", at(10), at(11));
    // deep recursion grows the thread's stack and call-info array while the walk reads them
    assert!(within(&exact.counts, "depth") >= 5000.0);
    assert!(exact.bytes.iter().all(|(stack, _)| stack.starts_with("startup")));
}

#[test]
fn sampling_estimates_the_bytes_and_counts_within_five_percent() {
    let exact = profile(1);
    let sampled = profile(256);
    for name in ["tables", "growth", "strings", "depth", "lines"] {
        let (attributed, measured) = (within(&sampled.bytes, name), sampled.measured[name]);
        println!("{name}: {attributed} sampled bytes of {measured} allocated ({:+.2} %)", (attributed / measured - 1.0) * 100.0);
        assert!((attributed - measured).abs() <= measured * 0.05, "{name}: {attributed} sampled, {measured} allocated");
    }
    for name in ["tables", "lines", "depth"] {
        let (counted, exact) = (within(&sampled.counts, name), within(&exact.counts, name));
        assert!((counted - exact).abs() <= exact * 0.05, "{name}: {counted} sampled, {exact} counted");
    }
}

/// ree's own engine step is not game code: the host-time profile books native time to the next Luau safepoint,
/// so the Animator, joint and physics steps the scheduler runs between a frame's handlers made a spring module's
/// PreSimulation and PostSimulation handlers look like 25-57 % of a frame. Inside `engine_step`
/// the same 100 ms goes to its own row; outside it the next Luau function carries it, as before.
#[test]
fn the_engine_step_has_its_own_row_in_the_host_time_profile() {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("ree-time-profile-{}-{stamp}", std::process::id()));
    let lua = Lua::new();
    assert!(luau_profile::begin_time_at(path.clone(), false, false));
    let guard = luau_profile::ProfileGuard;
    luau_profile::install(&lua);
    let work = lua.create_function(|_, ()| Ok(std::thread::sleep(std::time::Duration::from_millis(100)))).unwrap();
    lua.globals().set("sleep", work).unwrap();
    let handler = "local function handler() local n = 0 for i = 1, 200000 do n += i end return n end\nhandler()\n";
    // the engine's step first, as the scheduler runs it between two handlers
    luau_profile::engine_step("probe", || std::thread::sleep(std::time::Duration::from_millis(100)));
    lua.load(handler).set_name("=after").exec().unwrap();
    // the same time spent by a native function a handler called: the handler carries it
    lua.load(format!("sleep()\n{handler}")).set_name("=control").exec().unwrap();
    drop(guard);
    let text = std::fs::read_to_string(&path).expect("profile");
    let _ = std::fs::remove_file(&path);
    let weight = |needle: &str| -> u64 { text.lines().filter(|line| line.contains(needle)).map(|line| line.rsplit_once(' ').unwrap().1.parse::<u64>().unwrap()).sum() };
    let (engine, after, control) = (weight("(ree engine step: probe)"), weight(";handler after:"), weight(";handler control:") + weight("control:1"));
    assert!(engine >= 20, "the engine step booked {engine} ticks\n{text}");
    assert!(after * 4 < engine, "the handler after the step carries {after} ticks of it\n{text}");
    assert!(control >= 20, "the control, native time in the handler's own call, carries {control} ticks\n{text}");
}

/// The profiles share process-wide switches, and a guard's drop turns all of them off: a test that started one while
/// another ran its own (record/tests.rs's allocation profile, which took no lock) switched the work-clock profile
/// off under it, and that test read 0 microseconds profiled. Starting a profile now takes `TEST_LOCK` until
/// its guard drops, whichever test starts it.
#[test]
fn a_started_profile_holds_the_profile_lock_until_its_guard_drops() {
    use std::sync::TryLockError;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("ree-lock-profile-{}-{stamp}", std::process::id()));
    luau_profile::begin_allocations(path.clone(), 1024);
    let guard = luau_profile::ProfileGuard;
    let blocked = std::thread::spawn(|| matches!(luau_profile::TEST_LOCK.try_lock(), Err(TryLockError::WouldBlock))).join().unwrap();
    drop(guard);
    let kept = luau_profile::claimed();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("counts"));
    assert!(blocked, "another thread took the profile lock while a profile ran");
    assert!(!kept, "this thread still held the profile lock after the guard dropped");
}
