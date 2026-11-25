//! Harnesses with planted truths, run by the engine in this process (`ree run --profile ...`), and what the issues say
//! of each: one function holding the game's work, a sort with no comparator every frame, a comparator sort, a spike
//! the harness makes, setup work in the frame the steady phase is marked in, a game spike at frame 90, threads that
//! finish, repeat reads beside unchanged writes, a folder that kept profile.json alone, and a quiet run that must
//! report nothing.

// (the engine's service code calls mimalloc's mi_collect, which ree's binary links the same way)
extern crate mimalloc;

mod common;
#[path = "planted/engine.rs"]
mod engine;

use common::Temp;
use ree_profile_view::{
    app::{App, View},
    cli,
    issues::{Issue, Severity, Target},
    theme::ThemeKind,
};

/// Pattern of 16 `%a` followed by `%d`: fails every start position on an `"a"` string, doing linear C work on the
/// work clock (`17 * LUAU_MATCH_CLASS + LUAU_MATCH_STEP` ns/byte) with zero allocations and no builtin/cache noise.
const WORK_PATTERN: &str = "%a%a%a%a%a%a%a%a%a%a%a%a%a%a%a%a%d";
const WORK_NS_PER_BYTE: f64 = 17.0 * roblox_engine_emulator::cost::LUAU_MATCH_CLASS + roblox_engine_emulator::cost::LUAU_MATCH_STEP;

/// Length of `string.rep("a", n)` whose `string.match(_, WORK_PATTERN)` charges `ms` milliseconds on the work clock.
/// The last 16 start positions hit end-of-string early and average half a pattern (8 bytes less work), so add 8.
fn work_bytes(ms: f64) -> usize {
    (ms * 1e6 / WORK_NS_PER_BYTE).round().max(16.0) as usize + 8
}

/// Runs `work` over `items` across a bounded pool of scoped threads, preserving item order.
fn parallel_map<T: Send, R: Send, F: Fn(T) -> R + Sync>(items: Vec<T>, work: F) -> Vec<R> {
    let total = items.len();
    if total <= 1 {
        return items.into_iter().map(work).collect();
    }
    let workers = total.min(std::thread::available_parallelism().map_or(2, |n| n.get()).min(8));
    let queue = std::sync::Mutex::new(items.into_iter().enumerate());
    let results: std::sync::Mutex<Vec<Option<R>>> = std::sync::Mutex::new((0..total).map(|_| None).collect());
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let queue = &queue;
                let results = &results;
                let work = &work;
                scope.spawn(move || loop {
                    let next = queue.lock().unwrap().next();
                    let Some((index, item)) = next else { break };
                    let value = work(item);
                    results.lock().unwrap()[index] = Some(value);
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    results.into_inner().unwrap().into_iter().map(Option::unwrap).collect()
}

/// How close two figures in ms a frame must be to count as equal
const SAME_MS: f64 = 1e-9;

/// Asserts two figures in ms a frame are the same, saying which one they are when they are not
fn assert_same_ms(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < SAME_MS, "{what}: {actual} against {expected}");
}

#[test]
fn advised_numeric_argument_bound_covers_removed_conversion() {
    // The advice for math.abs(tostring(x)) is math.abs(x): the fix also removes the tostring call, work the site part
    // does not hold. The measured saving is larger than the site part and under the whole-code bound.
    let source = r#"local M = {}
function M.Step(x)
    local total = 0
    for i = 1, 12000 do
        total += math.abs(tostring(x))
    end
    return total
end
return M"#;
    let fixed_source = source.replace("math.abs(tostring(x))", "math.abs(x)");
    // The refitted clock charges C calls at 5.77 ns, library allocations and only executed cold instructions.
    let expected_site_ms = 0.146741700000;
    // tostring's fast call gives way to the C function at a call where a collector step is due (the heap is at its
    // threshold), and the frame's first such call pays that C function's cold first call. The collector sets its
    // threshold from the heap's size, so which of the 60 frames have such a call follows from the heap's size when the
    // run starts, and anything the engine adds to a VM's heap moves the count (56 and 59 of 60 have been recorded).
    // The figures are therefore those of a run with no such call, and what each one adds: 2497.05 ns, the difference
    // measured between runs of 56 and 59 (the cold first call's 2500 ns and the C call, less the fast call's own).
    let saving_without_debt_ms = 0.411801320630;
    let upper_without_debt_ms = 0.468370677296;
    let debt_decline_ms = 2497.05e-6 / 60.0;

    let temp = Temp::new("planted-numeric-argument");
    let handler = harness("Probe", "sink=M.Step(-3)");
    let (before_out, after_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "before", "Probe", source, &handler, "work,caches", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "fixed", "Probe", &fixed_source, &handler, "work,caches", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (before, after) = (open_out(&before_out), open_out(&after_out));
    let (found, _) = issues(&before);
    let declines = of(&found, "builtin declines");
    let abs_issue = declines.iter().find(|issue| issue.title.starts_with("math.abs")).unwrap();

    let debt_declines = |app: &App| {
        let builtins = app.profile().raw.builtins.as_ref().unwrap();
        builtins.iter().find(|builtin| builtin.name == "tostring").and_then(|builtin| builtin.declines.get("gc_debt").copied()).unwrap_or(0.0)
    };
    // (at most one a frame: a second in one frame would not pay the cold first call again; the fixed code calls no tostring)
    let declined = debt_declines(&before);
    assert!((1.0..=60.0).contains(&declined), "{declined} declines for a due collector step");
    assert_eq!(debt_declines(&after), 0.0);
    let expected_saving_ms = saving_without_debt_ms + declined * debt_decline_ms;
    let expected_upper_ms = upper_without_debt_ms + declined * debt_decline_ms;

    let saved_ms = code_per_frame(&before) - code_per_frame(&after);
    let upper_ms = abs_issue.upper_ms.expect("an advised edit has a whole-code bound");
    assert_same_ms(abs_issue.ms.unwrap(), expected_site_ms, "site part");
    assert_same_ms(saved_ms, expected_saving_ms, "measured saving");
    assert_same_ms(upper_ms, expected_upper_ms, "upper bound");
    assert_same_ms(upper_ms, code_per_frame(&before), "the bound is all the code's work");
    assert!(saved_ms <= upper_ms + SAME_MS, "saving {saved_ms} exceeds bound {upper_ms}");
    assert!(abs_issue.cost_text().contains("recorded work"));
    assert!(!abs_issue.advice.contains("nothing elsewhere"));
    assert_accounted(&before, &found);
    println!("numeric argument: site {expected_site_ms:.12}, A/B {saved_ms:.12}, upper {upper_ms:.12} ms/frame");
}

#[test]
fn advised_insert_bound_covers_removed_position_work() {
    // The position always equals the append position. Dropping it also removes M.Position's work, which the site part
    // leaves out. The refitted clock charges allocation and growth on both paths, so this site now has a saving.
    let source = r#"local M = {}
function M.Position(i)
    local sum = 0
    for j = 1, 5 do sum += j end
    return i + sum - 15
end
function M.Step(x)
    local values = {}
    for i = 1, 12000 do table.insert(values, M.Position(i), i) end
    return #values
end
return M"#;
    let fixed_source = source.replace("table.insert(values, M.Position(i), i)", "table.insert(values, i)");
    let frames = 60.0;

    let temp = Temp::new("planted-insert-argument");
    let handler = harness("Probe", "sink=M.Step(-3)");
    let (before_out, after_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "before", "Probe", source, &handler, "work,caches", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "fixed", "Probe", &fixed_source, &handler, "work,caches", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (before, after) = (open_out(&before_out), open_out(&after_out));
    let (found, _) = issues(&before);
    let insert = of(&found, "builtin declines")[0];
    let profile = before.profile();
    let recorded = profile.raw.builtins.as_ref().unwrap().iter().find(|builtin| builtin.name == "table.insert").unwrap();

    let site_net_ms = (recorded.decline_charged_us.unwrap() - recorded.fast_equivalent_us.unwrap()) / 1000.0 / frames;
    let saved_ms = code_per_frame(&before) - code_per_frame(&after);
    let upper_ms = insert.upper_ms.expect("an advised edit has a whole-code bound");
    assert!(site_net_ms > 0.0);
    assert_same_ms(insert.ms.unwrap(), site_net_ms, "site part");
    assert!(saved_ms > site_net_ms, "removing the position also saves its evaluation");
    assert!(!insert.no_saving, "the site part cannot decide the whole edit: {insert:#?}");
    assert_same_ms(upper_ms, code_per_frame(&before), "the bound is all the code's work");
    assert!(saved_ms <= upper_ms + SAME_MS, "saving {saved_ms} exceeds bound {upper_ms}");
    assert!(insert.cost_text().contains("recorded work"));
    assert!(insert.cost_text().contains("at most"));
    assert_accounted(&before, &found);
    println!("insert argument: site net {site_net_ms:.12}, A/B {saved_ms:.12}, upper {upper_ms:.12} ms/frame");
}

#[test]
fn reported_sort_bound_covers_argument_evaluation() {
    // Removing `table.sort(M.Build())` also removes the call to `Build`. The profile books `Build` to its own node,
    // beside the sort's subtree, so the sort's price (that subtree) is only part of what the edit saves.
    let source = r#"local M = {}
function M.Build()
    local t = table.create(10000)
    for i = 1, 10000 do t[i] = 10001 - i end
    return t
end
function M.Step()
    table.sort(M.Build())
    return 0
end
return M"#;
    let fixed_source = source.replace("table.sort(M.Build())", "");
    // The refit changes the sort's comparison price and Build's table.create/growth charges.
    let expected_subtotal_ms = 0.177079670000;
    let expected_saving_ms = 0.244118384668;

    let temp = Temp::new("planted-sort-arguments");
    let handler = harness("Probe", "sink=M.Step()");
    let (before_out, after_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "before", "Probe", source, &handler, "work,caches", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "fixed", "Probe", &fixed_source, &handler, "work,caches", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (before, after) = (open_out(&before_out), open_out(&after_out));

    let (found, _) = issues(&before);
    let sort = of(&found, "sort calls")[0];
    let subtotal_ms = sort.ms.unwrap();
    let upper_bound_ms = sort.upper_ms.unwrap();
    let all_code_ms = code_per_frame(&before);
    let saved_ms = all_code_ms - code_per_frame(&after);
    println!("sort argument: subtotal {subtotal_ms:.12}, upper {upper_bound_ms:.12}, A/B {saved_ms:.12} ms/frame");
    assert_same_ms(subtotal_ms, expected_subtotal_ms, "sort subtotal");
    assert_same_ms(saved_ms, expected_saving_ms, "sort saving");
    // The edit saves more than the sort's subtree and no more than the bound, which is all the code's work
    assert!(saved_ms <= upper_bound_ms + SAME_MS, "saving {saved_ms} exceeds bound {upper_bound_ms}");
    assert!((upper_bound_ms - all_code_ms).abs() < SAME_MS);
    assert!(sort.cost_text().contains("evaluation of its arguments"));
    assert_accounted(&before, &found);
}

#[test]
fn reported_environment_bound_covers_environment_calls() {
    // `getfenv()` makes the environment unsafe, so every `math.abs` call declines. Removing it restores the fast path
    // and also removes the `getfenv` calls, which the declined calls' price does not hold.
    let source = r#"local M = {}
function M.Step(x)
    local z = 0
    for i = 1, 12000 do
        getfenv()
        z += math.abs(x)
    end
    return z
end
return M"#;
    let fixed_source = source.replace("getfenv()", "");
    // Both C calls use the refitted price; fallback cold instructions are charged where they execute.
    let expected_subtotal_ms = 0.247499216667;
    let expected_saving_ms = 0.489434118340;

    let temp = Temp::new("planted-environment-calls");
    let handler = harness("Probe", "sink=M.Step(-3)");
    let (before_out, after_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "before", "Probe", source, &handler, "work,caches", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "fixed", "Probe", &fixed_source, &handler, "work,caches", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (before, after) = (open_out(&before_out), open_out(&after_out));

    let (found, _) = issues(&before);
    let declines = of(&found, "builtin declines");
    let builtin = declines.iter().find(|issue| issue.title.starts_with("math.abs")).unwrap();
    let subtotal_ms = builtin.ms.unwrap();
    let upper_bound_ms = builtin.upper_ms.unwrap();
    let all_code_ms = code_per_frame(&before);
    let saved_ms = all_code_ms - code_per_frame(&after);
    assert!((subtotal_ms - expected_subtotal_ms).abs() < SAME_MS);
    assert!((saved_ms - expected_saving_ms).abs() < SAME_MS);
    assert!(saved_ms <= upper_bound_ms + SAME_MS, "saving {saved_ms} exceeds bound {upper_bound_ms}");
    assert!((upper_bound_ms - all_code_ms).abs() < SAME_MS);
    let cost = builtin.cost_text();
    assert!(cost.contains("recorded work"));
    assert!(cost.contains("environment calls"));
    assert_accounted(&before, &found);
    println!("environment calls: subtotal {subtotal_ms:.12}, upper {upper_bound_ms:.12}, A/B {saved_ms:.12} ms/frame");
}

/// A module whose `Step` reads one part's Position 2000 times: one distinct read and 1999 repeats in the same call
const FRAME_READS: &str = r#"local p = Instance.new("Part")
local M = {}
function M.Step()
    local z = 0
    for i = 1, 2000 do
        if p.Position == nil then z += 1 else z += 2 end
    end
    return z
end
return M"#;

/// boundary.json as a run wrote it
fn read_boundary_json(out: &std::path::Path) -> serde_json::Value {
    let bytes = std::fs::read(out.join("boundary.json")).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The row of boundary.json's `by_member` for one member, such as `BasePart.Position`
fn boundary_member<'a>(boundary: &'a serde_json::Value, member: &str) -> &'a serde_json::Value {
    let members = boundary["by_member"].as_array().unwrap();
    members.iter().find(|row| row["member"] == member).unwrap()
}

/// A repeated Position read keeps the keyed instruction and getter's C call beside the engine's read price.
fn repeat_read_ms(reads: f64) -> f64 {
    use roblox_engine_emulator::cost;
    let keyed_read_ns = cost::INTERPRETED_OPS[15];
    let read_ns = cost::READ * 1e9 + cost::LUAU_CCALL + keyed_read_ns;
    reads * read_ns / 1e6
}

/// Checks a one-second run of `FRAME_READS` against the frames and rates it must report. The run has 60 rendered
/// frames, and the phase marked before the first one adds no 61st.
fn assert_boundary_frame_rates(out: &std::path::Path, app: &App) {
    let physical_frames = 60.0;
    let reads_per_frame = 2000.0;
    let distinct_per_frame = 1.0;
    let repeats_per_frame = 1999.0;
    let expected_repeat_ms = repeat_read_ms(repeats_per_frame);

    let boundary = read_boundary_json(out);
    let profile = app.profile();
    assert_eq!(profile.scope_frames(&profile.scope()), Some(physical_frames));
    assert_eq!(boundary["frames"].as_f64(), Some(physical_frames));
    let position = boundary_member(&boundary, "BasePart.Position");
    assert_eq!(position["count"].as_f64(), Some(physical_frames * reads_per_frame));
    assert_eq!(position["reads_per_frame"].as_f64(), Some(reads_per_frame));
    assert_eq!(position["distinct_per_frame"].as_f64(), Some(distinct_per_frame));
    assert_eq!(position["same_call_repeats_per_frame"].as_f64(), Some(repeats_per_frame));
    let summary = profile.raw.boundary.as_ref().unwrap();
    assert!(summary.contains("60 frames"), "{summary}");
    assert!(!summary.contains("61 frames"), "{summary}");

    let (found, _) = issues(app);
    let read = of(&found, "repeat reads")[0];
    assert!((read.ms.unwrap() - expected_repeat_ms).abs() < SAME_MS);
    assert!(read.title.contains("1999 a frame"));
    assert_accounted(app, &found);
    println!("boundary frame rates: {physical_frames} frames, {reads_per_frame} reads, {distinct_per_frame} distinct, {repeats_per_frame} repeats/frame");
}

/// Runs `FRAME_READS` for a second under `handler` and checks that the boundary profile counts only real frames
fn assert_frame_reads_are_counted_per_real_frame(case: &str, handler: &str) {
    let temp = Temp::new(case);
    let options = RunOptions { kinds: "work,boundary", seconds: "1", extra_arguments: &["--profile-phase", "frames"] };
    let (out, app) = run_with(&temp, "frames", "Probe", FRAME_READS, handler, &options);
    assert_boundary_frame_rates(&out, &app);
}

#[test]
fn reported_boundary_rates_exclude_pre_frame_marks() {
    // The harness names the phase `frames` itself, before any frame has rendered
    let handler = r#"return function(Context)
    local M = require(game.ReplicatedStorage.Game.Probe)
    Context.ProfilePhase("frames")
    local sink
    game:GetService("RunService").Heartbeat:Connect(function() sink = M.Step() end)
    task.wait(Context.Seconds)
end"#;
    assert_frame_reads_are_counted_per_real_frame("planted-pre-frame-mark", handler);
}

#[test]
fn reported_boundary_rates_exclude_automatic_frame_zero() {
    // The harness names no phase: the run command marks `frames` itself when the harness first replies, which is also
    // before any frame has rendered
    let handler = r#"return function(Context)
    local M = require(game.ReplicatedStorage.Game.Probe)
    local sink
    game:GetService("RunService").Heartbeat:Connect(function() sink = M.Step() end)
    task.wait(Context.Seconds)
end"#;
    assert_frame_reads_are_counted_per_real_frame("planted-automatic-frame-mark", handler);
}

#[test]
fn repeat_read_bounds_include_the_upvalue_receiver_loads_the_edit_removes() {
    // The edit keeps `p.Position` in a local, which also removes the loop's repeated loads of the upvalue `p`: work the
    // recorded subtotal of the repeat reads leaves out
    let source = |fixed: bool| {
        let (kept_local, value) = if fixed { ("local kept = p.Position", "kept") } else { ("", "p.Position") };
        format!(
            r#"local p = Instance.new("Part")
local M = {{}}
function M.Step()
    {kept_local}
    local z = 0
    for i = 1, 2000 do
        if {value} == nil then z += 1 else z += 2 end
    end
    return z
end
return M"#
        )
    };
    let expected_subtotal_ms = repeat_read_ms(1999.0);
    // Keeping the receiver in a local also removes 1999 GETUPVAL instructions (opcode 9).
    let expected_saving_ms = expected_subtotal_ms + 1999.0 * roblox_engine_emulator::cost::INTERPRETED_OPS[9] / 1e6;

    let temp = Temp::new("planted-receiver-bound");
    let handler = harness("Probe", "sink=M.Step()");
    let before_source = source(false);
    let after_source = source(true);
    let (before_out, after_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "before", "Probe", &before_source, &handler, "work,boundary", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "fixed", "Probe", &after_source, &handler, "work,boundary", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (before, after) = (open_out(&before_out), open_out(&after_out));

    let (found, _) = issues(&before);
    let read = of(&found, "repeat reads")[0];
    let subtotal_ms = read.ms.unwrap();
    let upper_bound_ms = read.upper_ms.unwrap();
    let saved_ms = code_per_frame(&before) - code_per_frame(&after);
    assert!((subtotal_ms - expected_subtotal_ms).abs() < SAME_MS);
    assert!((saved_ms - expected_saving_ms).abs() < SAME_MS);
    // The edit saves more than the subtotal and no more than the bound
    assert!(saved_ms <= upper_bound_ms);
    let cost = read.cost_text();
    assert!(cost.contains("recorded work"));
    assert!(cost.contains("including caller instructions"));
    assert_accounted(&before, &found);
    println!("repeat upvalue: subtotal {subtotal_ms:.12}, A/B {saved_ms:.12}, source bound 0..{upper_bound_ms:.12} ms/frame");
}

/// A module whose `Step` calls `string.sub(<text>, 2)` where the text lives in a different place each time
struct SubstringLayout {
    name: &'static str,
    source: &'static str,
    /// The variable the call reads, so the fix can add its end argument: `string.sub(text, 2, #text)`
    text: &'static str,
}

#[test]
fn substring_bounds_cover_upvalue_and_register_layouts() {
    let layouts = [
        SubstringLayout {
            name: "fresh",
            source: r#"local M = {}
local s = "abcdef"
function M.Step(frame, supplied)
    local z
    for i = 1, 5000 do
        s = tostring(i) .. "abcdef"
        z = string.sub(s, 2)
    end
    return z
end
return M"#,
            text: "s",
        },
        SubstringLayout {
            name: "parameter",
            source: r#"local M = {}
function M.Step(frame, s)
    local z = 0
    for i = 1, 10000 do z += #string.sub(s, 2) end
    return z
end
return M"#,
            text: "s",
        },
        SubstringLayout {
            name: "upvalue",
            source: r#"local M = {}
local text
function M.Step(frame, s)
    text = s
    local z = 0
    for i = 1, 10000 do z += #string.sub(text, 2) end
    return z
end
return M"#,
            text: "text",
        },
    ];
    let temp = Temp::new("planted-sub-layout-bounds");
    let handler = counting_harness("Probe", "frame+=1; sink=M.Step(frame,\"abcdef\")");
    let jobs: Vec<(String, String)> = layouts
        .iter()
        .flat_map(|layout| {
            let fixed = layout.source.replace(&format!("string.sub({}, 2)", layout.text), &format!("string.sub({}, 2, #{})", layout.text, layout.text));
            [(format!("{}-before", layout.name), layout.source.to_string()), (format!("{}-fixed", layout.name), fixed)]
        })
        .collect();
    let mut outs = parallel_map(jobs, |(case, source)| run_dir(&temp, &case, "Probe", &source, &handler, "work,caches", "1")).into_iter();
    for layout in layouts {
        let name = layout.name;
        let before = open_out(&outs.next().unwrap());
        let after = open_out(&outs.next().unwrap());

        let (found, _) = issues(&before);
        let declines = of(&found, "builtin declines");
        let issue = declines.into_iter().find(|issue| issue.title.starts_with("string.sub")).unwrap();
        let saved_ms = total_per_frame(&before) - total_per_frame(&after);
        let upper_bound_ms = issue.ms.unwrap();
        assert!(issue.at_most);
        assert!(issue.cost_text().starts_with("at most"));
        assert!(issue.advice.contains("receiver loads") && issue.advice.contains("zero"));
        assert!(saved_ms >= 0.0 && saved_ms <= upper_bound_ms + SAME_MS, "{name}: saved {saved_ms}, bound {upper_bound_ms}");
        println!("substring {name}: 0..{upper_bound_ms:.12} ms/frame; A/B {saved_ms:.12}");
        assert_accounted(&before, &found);
    }
}

/// A module that reads a table in a loop, the run's body that calls it, and the body after the fix
struct CacheCase {
    name: &'static str,
    source: &'static str,
    body: &'static str,
    fixed_body: &'static str,
}

#[test]
fn real_uncharged_cache_outcomes_are_estimates_with_calibration_spreads() {
    let cases = [
        CacheCase {
            name: "absent",
            source: r#"local M = {}
function M.Step(t)
    local z = 0
    for i = 1, 50000 do
        if t.x then z += 1 end
    end
    return z
end
return M"#,
            body: "sink=M.Step({})",
            fixed_body: "sink=M.Step({x=false})",
        },
        CacheCase {
            name: "metatable",
            source: r#"local M = {}
function M.Step(t)
    local z = 0
    for i = 1, 20000 do z += t.x end
    return z
end
return M"#,
            body: "sink=M.Step(setmetatable({},{__index={x=3}}))",
            fixed_body: "sink=M.Step({x=3})",
        },
        CacheCase {
            name: "shapes",
            source: r#"local M = {}
local tables = {}
for i = 1, 64 do
    local t = {}
    t["k" .. i] = 0
    t.x = 3
    tables[i] = t
end
function M.Step()
    local z = 0
    for i = 1, 50000 do
        local t = tables[i % 64 + 1]
        z += t.x
    end
    return z
end
return M"#,
            body: "sink=M.Step()",
            fixed_body: "sink=M.Step()",
        },
    ];
    let temp = Temp::new("planted-cache-estimates");
    let jobs: Vec<(String, String, String)> = cases
        .iter()
        .flat_map(|case| {
            // Only the shapes case has a source fix: it gives every table the same key order
            let fixed_source = if case.name == "shapes" { case.source.replace("t[\"k\" .. i]", "t[\"k\"]") } else { case.source.to_string() };
            [
                (format!("{}-before", case.name), case.source.to_string(), harness("Probe", case.body)),
                (format!("{}-fixed", case.name), fixed_source, harness("Probe", case.fixed_body)),
            ]
        })
        .collect();
    let mut outs = parallel_map(jobs, |(case, source, handler)| run_dir(&temp, &case, "Probe", &source, &handler, "work,caches", "1")).into_iter();
    for case in cases {
        let name = case.name;
        let before = open_out(&outs.next().unwrap());
        let after = open_out(&outs.next().unwrap());

        let (found, _) = issues(&before);
        let estimate = of(&found, "cache misses").into_iter().find(|issue| issue.uncharged > 0.01).unwrap();
        // The clock charged nothing for these outcomes, so there is no price, only an estimate inside its spread
        assert!(estimate.ms.is_none());
        assert_eq!(estimate.counted_ms, 0.0);
        let (low_ms, high_ms) = estimate.estimate_range.unwrap();
        assert!(low_ms <= estimate.uncharged && estimate.uncharged <= high_ms, "{estimate:#?}");
        let cost = estimate.cost_text();
        assert!(cost.contains("estimated") && cost.contains("calibration"));
        assert!(!estimate.advice.contains("nothing elsewhere"));
        // And fixing it saves no work on the clock
        let saved_ms = code_per_frame(&before) - code_per_frame(&after);
        assert!(saved_ms.abs() < SAME_MS, "{name}: clock saving {saved_ms}");
        println!("cache {name}: estimated {:.12}, spread [{low_ms:.12}, {high_ms:.12}], clock saving {saved_ms:.12} ms/frame", estimate.uncharged);
        assert_accounted(&before, &found);
    }
}

#[test]
fn phase_split_repeat_reads_use_physical_frames_and_the_executing_owner() {
    // Each member is read through a small helper the compiler inlines into `Step`: the boundary records `Step` as the
    // executing function, and the profile's 60 frames count once though the phase mark splits each of them
    let members = ["Position", "Size", "Anchored", "CFrame", "Color", "Material", "Name", "Parent", "Transparency", "Orientation"];
    let physical_frames = 60.0;
    let temp = Temp::new("planted-phase-inline-owner");
    let handler = harness("Probe", "Context.ProfilePhase(\"a\"); sink=M.Step(); Context.ProfilePhase(\"b\")");
    let options = RunOptions { kinds: "work,boundary", seconds: "1", extra_arguments: &["--profile-phase", "a"] };
    let apps = parallel_map(members.to_vec(), |member| {
        let source = format!(
            r#"local M = {{}}
local p = Instance.new("Part")
p.Parent = workspace
local function Read(v) return v.{member} end
function M.Step()
    local z = 0
    for i = 1, 2000 do
        if Read(p) ~= nil then z += 1 end
    end
    return z
end
return M"#
        );
        (member, run_dir_with(&temp, member, "Probe", &source, &handler, &options))
    });
    for (member, out) in apps {
        let app = open_out(&out);
        let (found, _) = issues(&app);
        let repeats = of(&found, "repeat reads");
        assert_eq!(repeats.len(), 1, "{found:#?}");
        let issue = repeats[0];
        let p = app.profile();
        assert_eq!(p.scope_frames(&p.scope()), Some(physical_frames));
        assert_eq!(p.boundary.as_ref().unwrap().frames, physical_frames);
        let Target::Line(owner, _) = issue.target else { panic!("{issue:#?}") };
        assert_eq!(p.funcs[owner as usize].name, "Step");
        assert!(issue.ms.unwrap() > 0.04, "{issue:#?}");
        assert!(issue.upper_ms.unwrap() >= issue.ms.unwrap());
        assert!(issue.cost_text().contains("including caller instructions"));
        assert_accounted(&app, &found);
        println!("phase/inlined {member}: {:.12} ms/frame subtotal, physical frames {physical_frames}", issue.ms.unwrap());
    }
}

#[test]
fn all_205_builtin_sites_contribute_to_the_source_edits_price() {
    // One line holds 205 compiled copies of the call. The profile keeps every site (it once kept the first 200)
    let site_count = 205;
    let declined_calls_in_the_run = 1_230_000.0;
    let call = "z+=math.abs(s)";
    let line_of_calls = vec![call; site_count].join(";");
    let source = format!(
        r#"local M = {{}}
function M.Step(s)
    local z = 0
    for i = 1, 100 do {line_of_calls} end
    return z
end
return M"#
    );
    let temp = Temp::new("planted-all-sites");
    // A numeric string is declined by the fast path; the fix passes the number itself
    let before_handler = harness("Probe", "sink=M.Step(\"3\")");
    let fixed_handler = harness("Probe", "sink=M.Step(3)");
    let (before_out, fixed_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "before", "Probe", &source, &before_handler, "work,caches", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "fixed", "Probe", &source, &fixed_handler, "work,caches", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (before, fixed) = (open_out(&before_out), open_out(&fixed_out));

    let (found, _) = issues(&before);
    let issue = of(&found, "builtin declines")[0];
    let builtins = before.profile().raw.builtins.as_ref().unwrap();
    let math_abs = builtins.iter().find(|builtin| builtin.name == "math.abs").unwrap();
    assert_eq!(math_abs.sites.len(), site_count);
    let declines_at_sites: f64 = math_abs.sites.iter().map(|site| site.declines.values().sum::<f64>()).sum();
    assert_eq!(declines_at_sites, declined_calls_in_the_run);
    let saved_ms = code_per_frame(&before) - code_per_frame(&fixed);
    assert!((issue.ms.unwrap() - saved_ms).abs() < SAME_MS, "{saved_ms}, {issue:#?}");
    println!("205 sites: priced {:.12}, A/B {saved_ms:.12} ms/frame", issue.ms.unwrap());
    assert_accounted(&before, &found);
}

#[test]
fn unsafe_environment_fixes_name_the_combination_and_share_fallback_work() {
    // `getfenv()` makes the environment unsafe, so every `math.abs` call is declined before its arguments are looked at.
    // Restoring the environment alone cannot give the saving: a string argument is declined again for its type
    let source = r#"local M = {}
function M.Step(x)
    local z = 0
    for i = 1, 12000 do z += math.abs(x) end
    return z
end
return M"#;
    let unsafe_source = format!("getfenv()\n{source}");
    let temp = Temp::new("planted-unsafe-combination");
    let pairs = [("numeric", "-3"), ("string", "\"3\"")];
    let jobs: Vec<(String, &str, String)> = pairs
        .iter()
        .flat_map(|&(name, argument)| {
            let handler = harness("Probe", &format!("sink=M.Step({argument})"));
            [
                (format!("{name}-before"), unsafe_source.as_str(), handler.clone()),
                (format!("{name}-fixed"), source, handler),
            ]
        })
        .collect();
    let mut outs = parallel_map(jobs, |(case, src, handler)| run_dir(&temp, &case, "Probe", src, &handler, "work,caches", "1")).into_iter();
    let mut numeric_fast_ms: Option<f64> = None;
    for (name, _) in pairs {
        let before = open_out(&outs.next().unwrap());
        let fixed = open_out(&outs.next().unwrap());

        let (found, _) = issues(&before);
        let builtin = of(&found, "builtin declines")[0];
        let cache = of(&found, "cache misses").into_iter().find(|issue| issue.ms.is_some()).unwrap();
        // The decline and the cache lookup in its fallback share work, which counts once in their union
        let node_work: Vec<f64> = before.profile().raw_nodes().iter().map(|node| node.own).collect();
        let mut union = builtin.work.clone();
        union.union(&cache.work);
        let union_ms = union.total(&node_work) / builtin.reported_frames;
        assert!((union_ms - builtin.ms.unwrap()).abs() < 1e-12, "{builtin:#?}\n{cache:#?}");
        assert!(builtin.evidence.iter().any(|line| line.contains("overlaps cache misses")));
        assert!(builtin.at_most);
        assert!(builtin.advice.contains("combination") && builtin.advice.contains("types"));

        let saved_ms = code_per_frame(&before) - code_per_frame(&fixed);
        assert!(saved_ms <= builtin.ms.unwrap() + SAME_MS);
        if name == "string" {
            // Restoring the environment still leaves the string declined for its type...
            let still_declined_for_types = fixed.profile().raw.builtins.as_ref().unwrap().iter().any(|builtin| builtin.declines.get("types").copied().unwrap_or(0.0) > 0.0);
            assert!(still_declined_for_types);
            assert!(saved_ms < builtin.ms.unwrap() * 0.5);
            // ...and the bound is the saving of fixing both
            let combined_saving_ms = code_per_frame(&before) - numeric_fast_ms.unwrap();
            assert!((combined_saving_ms - builtin.ms.unwrap()).abs() < SAME_MS);
            println!("unsafe string combined environment/type fix: {combined_saving_ms:.12} ms/frame");
        } else {
            numeric_fast_ms = Some(code_per_frame(&fixed));
        }
        println!("unsafe {name}: combination bound {:.12}, environment-only A/B {saved_ms:.12}, cache {:.12} ms/frame", builtin.ms.unwrap(), cache.ms.unwrap());
        assert_accounted(&before, &found);
    }
}

/// A module with one frame that does a lot of work, and what its fix removes
struct IsolatedPeak {
    name: &'static str,
    source: &'static str,
    /// The issue kind the peak belongs to
    kind: &'static str,
    /// The text the fix replaces, and what it puts there
    fix_from: &'static str,
    fix_to: &'static str,
}

#[test]
fn isolated_builtin_and_sort_peaks_survive_without_heavy_history() {
    let peaks = [
        IsolatedPeak {
            name: "byte",
            source: r#"local M = {}
function M.Step(frame, s)
    local z = 0
    if frame == 30 then
        for i = 1, 50000 do z += string.byte(s) end
    end
    return z
end
return M"#,
            kind: "builtin declines",
            fix_from: "string.byte(s)",
            fix_to: "string.byte(s,1)",
        },
        IsolatedPeak {
            name: "sort",
            source: r#"local M = {}
local t = table.create(50000)
function M.Step(frame, s)
    if frame == 30 then
        for i = 1, 50000 do t[i] = 50001 - i end
        table.sort(t)
    end
    return #t
end
return M"#,
            kind: "sort calls",
            fix_from: "table.sort(t)",
            fix_to: "",
        },
    ];
    let peak_frame_floor_ms = 0.41666666;
    let mean_ceiling_ms = 0.01;
    let temp = Temp::new("planted-complete-peaks");
    let handler = counting_harness("Probe", "frame+=1; sink=M.Step(frame,\"abcdef\")");
    let jobs: Vec<(String, String)> = peaks
        .iter()
        .flat_map(|peak| {
            let fixed = peak.source.replace(peak.fix_from, peak.fix_to);
            [(peak.name.to_string(), peak.source.to_string()), (format!("{}-fixed", peak.name), fixed)]
        })
        .collect();
    let mut outs = parallel_map(jobs, |(case, source)| run_dir(&temp, &case, "Probe", &source, &handler, "work,caches", "10")).into_iter();
    for peak in peaks {
        let IsolatedPeak { name, kind, .. } = peak;
        // (ten seconds of frames: the one heavy frame is a tiny share of the mean, and no heavy-frame history is kept)
        let app = open_out(&outs.next().unwrap());
        let after = open_out(&outs.next().unwrap());
        let (found, _) = issues(&app);
        let issue = of(&found, kind)[0];
        assert!(app.profile().raw.timeline.as_ref().unwrap().node_frames.is_empty());
        assert!(issue.ms.unwrap() < mean_ceiling_ms);
        assert!(issue.frame_ms.unwrap() > peak_frame_floor_ms, "{issue:#?}");
        assert_eq!(issue.active_frames, 1);

        let saved_ms = total_per_frame(&app) - total_per_frame(&after);
        let upper_bound_ms = issue.upper_ms.or(issue.ms).unwrap();
        assert!(saved_ms > 0.0 && saved_ms <= upper_bound_ms + SAME_MS, "{name}: {saved_ms} exceeds {upper_bound_ms}");
        println!("isolated {name}: {:.12} ms/frame; peak {:.12} ms", issue.ms.unwrap(), issue.frame_ms.unwrap());
        assert_accounted(&app, &found);
    }
}

/// A harness that waits a frame, marks the steady phase and connects `body` as its frame handler, as a clip's does
fn harness(require: &str, body: &str) -> String {
    format!(
        "return function(Context)\n\tlocal RunService = game:GetService(\"RunService\")\n\tlocal M = require(game.ReplicatedStorage.Game.{require})\n\ttask.wait()\n\tContext.ProfilePhase(\"steady\")\n\tlocal sink = 0\n\tRunService.Heartbeat:Connect(function()\n\t\t{body}\n\tend)\n\ttask.wait(Context.Seconds)\nend\n"
    )
}

/// Runs `harness` with the game module `name` (`source`) mounted at ReplicatedStorage.Game for `seconds`, with
/// `--profile <kinds>`: the profile's folder, opened
fn run(temp: &Temp, case: &str, name: &str, source: &str, harness: &str, kinds: &str, seconds: &str) -> (std::path::PathBuf, App) {
    let options = RunOptions { kinds, seconds, extra_arguments: &[] };
    run_with(temp, case, name, source, harness, &options)
}

/// Runs [`run`] and returns only the profile directory (Send across worker threads; open with [`open_out`]).
fn run_dir(temp: &Temp, case: &str, name: &str, source: &str, harness: &str, kinds: &str, seconds: &str) -> std::path::PathBuf {
    let options = RunOptions { kinds, seconds, extra_arguments: &[] };
    run_dir_with(temp, case, name, source, harness, &options)
}

fn open_out(out: &std::path::Path) -> App {
    cli::open(&[out.to_path_buf()], ThemeKind::Dark).unwrap()
}

/// What a planted run is told besides its harness and game module
struct RunOptions<'a> {
    /// The profile kinds (`--profile`)
    kinds: &'a str,
    /// How long the run lasts (`--seconds`)
    seconds: &'a str,
    /// Any other command line arguments, such as `--fps 120`
    extra_arguments: &'a [&'a str],
}

/// [`run_with`] returning only the profile directory so callers can run multiple engines in parallel.
fn run_dir_with(temp: &Temp, case: &str, name: &str, source: &str, harness: &str, options: &RunOptions) -> std::path::PathBuf {
    let dir = temp.path().join(case);
    std::fs::create_dir_all(dir.join("game")).unwrap();
    std::fs::write(dir.join("game").join(format!("{name}.luau")), source).unwrap();
    let file = dir.join(format!("{case}.luau"));
    std::fs::write(&file, harness).unwrap();
    let out = dir.join("profile");
    let mount = format!("ReplicatedStorage/Game={}", dir.join("game").to_string_lossy().replace('\\', "/"));
    let mut args: Vec<String> = ["--harness", file.to_str().unwrap(), "--mount", &mount, "--seconds", options.seconds, "--profile", options.kinds, "--profile-out", out.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    args.extend(options.extra_arguments.iter().map(|arg| arg.to_string()));
    args.extend(["--max-wall-seconds", "0"].map(str::to_string));
    // (on a thread with a service worker's stack, as the ree binary runs its commands; the test's folder for its checkout, so the
    // run's state, `.cache/record`, goes with the test: engine.rs run_engine_with)
    let repo = temp.path().to_path_buf();
    let code = std::thread::Builder::new().stack_size(64 << 20).spawn(move || roblox_engine_emulator::record::run::cli_in_process(&args, &roblox_engine_emulator::launch::Host { repo, ..Default::default() }, false)).unwrap().join().unwrap();
    assert_eq!(code, 0, "{case}");
    out
}

/// [`run`] with command line arguments of its own
fn run_with(temp: &Temp, case: &str, name: &str, source: &str, harness: &str, options: &RunOptions) -> (std::path::PathBuf, App) {
    let out = run_dir_with(temp, case, name, source, harness, options);
    let app = open_out(&out);
    (out, app)
}

fn issues(app: &App) -> (Vec<Issue>, Vec<String>) {
    let found = app.issues();
    (found.0.clone(), found.1.clone())
}

fn of<'a>(issues: &'a [Issue], kind: &str) -> Vec<&'a Issue> {
    issues.iter().filter(|issue| issue.kind == kind).collect()
}

fn hot(issues: &[Issue]) -> Vec<&Issue> {
    issues.iter().filter(|issue| issue.kind.starts_with("hot")).collect()
}

const HOT: &str = "local M = {}\nlocal data = {}\nfor i = 1, 1000 do\n\tdata[i] = i * 0.5\nend\n\nfunction M.Heavy()\n\tlocal s = 0\n\tfor r = 1, 20 do\n\t\tfor i = 1, #data do\n\t\t\ts = s + data[i] * 1.0001 + r\n\t\tend\n\tend\n\treturn s\nend\n\nfunction M.Light()\n\tlocal s = 0\n\tfor i = 1, 50 do\n\t\ts = s + i\n\tend\n\treturn s\nend\n\nfunction M.Step()\n\treturn M.Heavy() + M.Light() + M.Light()\nend\n\nreturn M\n";

#[test]
fn one_function_holding_the_games_work_is_named() {
    // Heavy, 20,000 steps a frame, against two Lights of 50: 99.7% of the game's work, its line 11 nearly all of it
    let temp = Temp::new("planted-hot");
    let (_, app) = run(&temp, "hot", "Hot", HOT, &harness("Hot", "sink += M.Step()"), "all", "2");
    let (found, _) = issues(&app);
    let hot = hot(&found);
    assert_eq!(hot.len(), 1, "{found:#?}");
    assert_eq!(hot[0].kind, "hot line");
    let share = format!("{:.1}% of the game's work", hot[0].ms.unwrap() / code_per_frame(&app) * 100.0);
    assert!(hot[0].title.starts_with("Heavy Hot:7:") && hot[0].evidence.iter().any(|line| line.contains(&share)), "{}", hot[0].title);
    assert!(priced_sum(&found) <= code_per_frame(&app) + 1e-9, "{found:#?}");
    let ms = hot[0].ms.unwrap();
    assert!(ms > 0.05 && ms < 0.2, "{ms} ms a frame");
    assert_eq!(hot[0].severity, Severity::of(ms, app.profile().budget_ms()));
    // The VM exports per-phase instruction work, so the line's evidence uses the selected phase too.
    assert!(hot[0].evidence.iter().any(|line| line.starts_with("its lines: Hot:11")), "{:?}", hot[0].evidence);
}

const SORT: &str = "local M = {}\nlocal values = {}\nfor i = 1, 5000 do\n\tvalues[i] = (i * 7919) % 10007\nend\n\nfunction M.Rank()\n\tfor i = 1, #values do\n\t\tvalues[i] = (values[i] * 31 + i) % 10007\n\tend\n\ttable.sort(values)\n\treturn values[1]\nend\n\nfunction M.Other()\n\tlocal s = 0\n\tfor i = 1, 3000 do\n\t\ts += i % 7\n\tend\n\treturn s\nend\n\nreturn M\n";

#[test]
fn a_sort_with_no_comparator_is_named_where_the_profile_records_c_calls_and_said_unchecked_where_not() {
    // table.sort(values) of 5,000 numbers every frame, a third of the game's work: this build records a C function only
    // when Luau runs under it, so the sort is not found as a sort, and the issues say so; the hot function that makes it
    // lists the sort's line among its own (the C call's work is booked there)
    let temp = Temp::new("planted-sort");
    let (_, app) = run(&temp, "sort2", "Sort2", SORT, &harness("Sort2", "sink += M.Rank() + M.Other()"), "all", "2");
    let (found, unchecked) = issues(&app);
    let c_rows = app.profile().raw.work.as_ref().is_some_and(|work| work.c_calls);
    if c_rows {
        // (a ree that records C calls at their call sites: the sort, at its line)
        let sorts = of(&found, "sort calls");
        assert_eq!(sorts.len(), 1, "{found:#?}");
        assert!(matches!(sorts[0].target, Target::Line(_, 11)), "{:?}", sorts[0].target);
    } else {
        assert!(of(&found, "sort calls").is_empty(), "{found:#?}");
        assert!(unchecked.iter().any(|note| note.starts_with("sorts without a comparator: need C call rows")), "{unchecked:?}");
        let rank = hot(&found);
        assert_eq!(rank.len(), 1, "{found:#?}");
        assert!(rank[0].title.contains("Rank Sort2:7"), "{}", rank[0].title);
        assert!(rank[0].evidence.iter().any(|line| line.contains("Sort2:11 ") && line.contains("CALLFB 120")), "{:?}", rank[0].evidence);
    }
}

const SORTS: &str = "local M = {}\nlocal entries = {}\nfor i = 1, 300 do\n\tentries[i] = { id = i, score = (i * 7919) % 1000 }\nend\n\nfunction M.Board()\n\tfor i, e in entries do\n\t\te.score = (e.score + i) % 1000\n\tend\n\ttable.sort(entries, function(a, b)\n\t\treturn a.score > b.score\n\tend)\n\treturn entries[1].id\nend\n\nlocal pair = { 2, 1 }\nfunction M.Pair()\n\tpair[1], pair[2] = pair[2], pair[1]\n\ttable.sort(pair)\n\treturn pair[1]\nend\n\nfunction M.Work()\n\tlocal s = 0\n\tfor i = 1, 4000 do\n\t\ts += i % 7\n\tend\n\treturn s\nend\n\nreturn M\n";

#[test]
fn a_comparator_sort_is_named_once_and_its_calls_are_not_taken_for_runs() {
    // Board sorts 300 entries a frame with a comparator (39% of the game's work): one sort issue, the comparator's work
    // in it rather than an issue of its own, and its calls said to be counted once a sort
    let temp = Temp::new("planted-sorts");
    let (_, mut app) = run(&temp, "sorts", "Sorts", SORTS, &harness("Sorts", "sink += M.Board() + M.Pair() + M.Work()"), "all", "2");
    let (found, _) = issues(&app);
    let sorts = of(&found, "sort calls");
    assert_eq!(sorts.len(), 1, "{found:#?}");
    assert!(sorts[0].title.starts_with("Sort calls: Board  Sorts:7"), "{}", sorts[0].title);
    assert!(sorts[0].evidence[1].contains("table.sort ran 120 times") && sorts[0].evidence[1].contains("1.0 a frame"), "{:?}", sorts[0].evidence);
    let invocations = app.profile().raw.work.as_ref().is_some_and(|work| work.c_invocations);
    if !invocations {
        assert!(sorts[0].evidence[2].ends_with("counted once a sort, not once a run (needs invocation counts)"), "{:?}", sorts[0].evidence);
    }
    assert!(hot(&found).iter().filter(|issue| issue.title.contains("anonymous")).all(|issue| issue.overlap > 0.0), "comparator removal overlaps sort removal: {found:#?}");
    // and the Calls view says the same of the comparator
    if !invocations {
        assert!(app.select("Sorts:11"));
        app.view = View::Calls;
        let text = ree_profile_view::export::text(&cli::screen(&mut app, 200, 30));
        assert!(text.contains("calls, counted once a C call (needs invocation counts)"), "{text}");
    }
}

const SMALL: &str = "local M = {}\n\nfunction M.Step(n)\n\tlocal s = 0\n\tfor i = 1, n do\n\t\ts += i % 3\n\tend\n\treturn s\nend\n\nreturn M\n";

const SPIKE: &str = "local M = {}\nlocal frame = 0\n\nfunction M.Rebuild()\n\tlocal s = 0\n\tfor i = 1, 400000 do\n\t\ts += i % 13\n\tend\n\treturn s\nend\n\nfunction M.Tick()\n\tframe += 1\n\tlocal s = 0\n\tfor i = 1, 2000 do\n\t\ts += i % 5\n\tend\n\tif frame == 90 then\n\t\ts += M.Rebuild()\n\tend\n\treturn s\nend\n\nreturn M\n";

#[test]
fn a_game_spike_is_named_and_the_harnesss_or_the_setups_are_not() {
    let temp = Temp::new("planted-spikes");
    // the harness scans 600,000 steps on frame 60 (4.4 ms): its own work, no game spike
    let scan = "frame += 1\n\t\tsink += M.Step(300)\n\t\tif frame == 60 then\n\t\t\tfor i = 1, 600000 do\n\t\t\t\tsink += i % 11\n\t\t\tend\n\t\tend";
    let source = harness("Small", scan).replace("local sink = 0", "local sink, frame = 0, 0");
    // heavy setup, then the steady mark in the same frame: the setup's heavy work is the setup phase's row, not the steady
    // phase's first frame
    let setup = "return function(Context)\n\tlocal RunService = game:GetService(\"RunService\")\n\tlocal M = require(game.ReplicatedStorage.Game.Small)\n\ttask.wait()\n\tContext.ProfilePhase(\"setup\")\n\tlocal world = {}\n\tfor i = 1, 1200000 do\n\t\tworld[i] = i % 17\n\tend\n\tContext.ProfilePhase(\"steady\")\n\tlocal sink = 0\n\tRunService.Heartbeat:Connect(function()\n\t\tsink += M.Step(300) + #world\n\tend)\n\ttask.wait(Context.Seconds)\nend\n";
    let game_spike_handler = harness("Spike", "sink += M.Tick()");
    let (out_harness, out_setup, out_game) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "harness-spike", "Small", SMALL, &source, "all", "2"));
        let h2 = scope.spawn(|| run_dir(&temp, "same-mark", "Small", SMALL, setup, "all", "1"));
        let h3 = scope.spawn(|| run_dir(&temp, "game-spike", "Spike", SPIKE, &game_spike_handler, "all", "2"));
        (h1.join().unwrap(), h2.join().unwrap(), h3.join().unwrap())
    });
    let (app_harness, app_setup, app_game) = (open_out(&out_harness), open_out(&out_setup), open_out(&out_game));
    let (found, _) = issues(&app_harness);
    assert!(of(&found, "one-off").is_empty() && of(&found, "frame hitch").is_empty(), "{found:#?}");
    let (found, _) = issues(&app_setup);
    assert!(of(&found, "one-off").is_empty() && of(&found, "frame hitch").is_empty(), "{found:#?}");
    let p = app_setup.profile();
    let timeline = p.raw.timeline.as_ref().unwrap();
    let (phases, work) = (timeline.values("phase").unwrap(), timeline.values("work_ms").unwrap());
    let setup_phase = p.raw.phases.iter().position(|phase| phase == "setup").unwrap() as f64;
    let setup_work: f64 = phases.iter().zip(&work).filter(|(phase, _)| **phase == setup_phase).map(|(_, work)| *work).sum();
    assert!(setup_work > 10.0, "the setup's work is the setup phase's: {setup_work} ms");
    // the game's frame 90 rebuilds (2.6 ms against 13 µs): Rebuild's one-off, named
    let (found, _) = issues(&app_game);
    let once = of(&found, "one-off");
    assert_eq!(once.len(), 1, "{found:#?}");
    assert_eq!(once[0].once, Some(90.0));
    assert!(once[0].title.starts_with("Rebuild Spike:4:"), "{}", once[0].title);
    // (its hitch, the severity: the frame's 2.6 ms fits in a 16.67 ms frame)
    assert_eq!(once[0].severity, Severity::Info);
}

const SPAWNER: &str = "local M = {}\nfunction M.Step()\n\tfor _ = 1, 5 do\n\t\ttask.spawn(function()\n\t\t\tlocal x = 0\n\t\t\tfor i = 1, 10 do\n\t\t\t\tx += i\n\t\t\tend\n\t\tend)\n\tend\nend\nreturn M\n";

#[test]
fn threads_that_finish_are_never_a_leak() {
    // five threads a frame that end at once: the timeline counts them until the collector sweeps them
    let temp = Temp::new("planted-threads");
    let handler = harness("Spawner", "M.Step()");
    let (out_wt, out_all) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "work-timeline", "Spawner", SPAWNER, &handler, "work,timeline", "2"));
        let h2 = scope.spawn(|| run_dir(&temp, "all", "Spawner", SPAWNER, &handler, "all", "2"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (app_wt, app_all) = (open_out(&out_wt), open_out(&out_all));
    let (found, unchecked) = issues(&app_wt);
    assert!(found.iter().all(|issue| !issue.title.starts_with("Threads")), "{found:#?}");
    assert!(unchecked.iter().any(|note| note.starts_with("threads keep growing: needs heap")), "{unchecked:?}");
    let (found, _) = issues(&app_all);
    assert!(found.iter().all(|issue| !issue.title.starts_with("Threads")), "{found:#?}");
}

const READS: &str = "local M = {}\n\nfunction M.Follow(part)\n\tlocal s = 0\n\tfor _ = 1, 2000 do\n\t\ts += part.Position.X\n\tend\n\treturn s\nend\n\nfunction M.Fade(part)\n\tfor _ = 1, 2000 do part.Transparency = 0.5 end\nend\n\nreturn M\n";

#[test]
fn repeat_reads_and_unchanged_writes_rank_by_the_work_they_cost() {
    // Follow reads a part's Position 12 times a frame (11 again), Fade writes Transparency = 0.5 every frame though it is
    // 0.5: both priced from one `all` profile at what removing them saves, and the costlier ranks first
    let temp = Temp::new("planted-reads");
    let source = "return function(Context)\n\tlocal RunService = game:GetService(\"RunService\")\n\tlocal M = require(game.ReplicatedStorage.Game.Reads)\n\tlocal part = Instance.new(\"Part\")\n\tpart.Anchored = true\n\tpart.Parent = workspace\n\ttask.wait()\n\tContext.ProfilePhase(\"steady\")\n\tlocal sink = 0\n\tRunService.Heartbeat:Connect(function()\n\t\tsink += M.Follow(part)\n\t\tM.Fade(part)\n\tend)\n\ttask.wait(Context.Seconds)\nend\n";
    let (_, app) = run(&temp, "reads", "Reads", READS, source, "all", "2");
    let (found, unchecked) = issues(&app);
    assert!(unchecked.iter().all(|note| !note.contains("need boundary")), "all holds the boundary profile: {unchecked:?}");
    let (reads, writes) = (of(&found, "repeat reads"), of(&found, "unchanged writes"));
    assert_eq!((reads.len(), writes.len()), (1, 1), "{found:#?}");
    let at = |issue: &Issue| found.iter().position(|other| other.title == issue.title).unwrap();
    let (first, second) = if writes[0].ms.unwrap() > reads[0].ms.unwrap() { (writes[0], reads[0]) } else { (reads[0], writes[0]) };
    assert!(at(first) < at(second), "{found:#?}");
}

const ALLOC: &str = "local M = {}\n\nfunction M.Points(n)\n\tlocal keep = 0\n\tfor i = 1, n do\n\t\tlocal p = { x = i, y = i * 2 }\n\t\tkeep += p.x\n\tend\n\treturn keep\nend\n\nreturn M\n";

#[test]
fn a_folder_that_kept_profile_json_alone_says_what_it_could_not_check() {
    // Points makes 200 small tables a frame: found with the folded stacks, and said unchecked (never silently gone) when a
    // folder keeps profile.json alone
    let temp = Temp::new("planted-json");
    let (out, app) = run(&temp, "alloc", "Alloc", ALLOC, &harness("Alloc", "sink += M.Points(tonumber(\"200\"))"), "all", "2");
    let (found, _) = issues(&app);
    assert!(!of(&found, "steady allocation").is_empty(), "{found:#?}");
    let kept = temp.path().join("kept");
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::copy(out.join("profile.json"), kept.join("profile.json")).unwrap();
    let app = cli::open(&[kept], ThemeKind::Dark).unwrap();
    let (found, unchecked) = issues(&app);
    assert!(of(&found, "steady allocation").is_empty(), "{found:#?}");
    assert!(unchecked.iter().any(|note| note.starts_with("steady allocation per site: needs the folded allocation stacks")), "{unchecked:?}");
}

const FAST: &str = "local M = {}\n\nfunction M.Step(n)\n\tlocal s = 0\n\tfor i = 1, n do\n\t\ts += math.floor(i * 0.5) + math.abs(-i) + math.max(i, 3)\n\t\ts += string.byte(\"abc\", 2)\n\tend\n\treturn s\nend\n\nreturn M\n";

#[test]
fn a_quiet_run_reports_nothing() {
    // plain builtin calls, 500 a frame, every one on its fast path: nothing to fix, however its shares fall
    let temp = Temp::new("planted-quiet");
    let (_, app) = run(&temp, "fast", "Fast", FAST, &harness("Fast", "sink += M.Step(tonumber(\"500\"))"), "all", "2");
    let (found, _) = issues(&app);
    assert!(found.iter().all(|issue| issue.kind == "work below floor"), "{found:#?}");
}

fn burst_module() -> String {
    let burst = work_bytes(25.6);
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local frame, _b = 0, string.rep("a", {burst})

function M.Burst()
	return _m(_b, _p) and 1 or 0
end

function M.Tick()
	frame += 1
	local s = 0
	for i = 1, 2000 do
		s += i % 5
	end
	if frame == 60 then
		s += M.Burst()
	end
	return s
end

return M
"#
    )
}

#[test]
fn a_one_off_is_its_own_issue_never_steady_work() {
    // Burst runs once, in frame 60 (about 25 ms, most of the phase's work): a one-off issue names it, its size not priced
    // a frame, graded by its hitch (the frame overruns by half a frame or more); the frame's hitch names it too; and it is
    // never advised as a function to run less often every frame
    let temp = Temp::new("planted-burst");
    let (_, app) = run(&temp, "burst", "Burst", &burst_module(), &harness("Burst", "sink += M.Tick()"), "all", "2");
    let (found, _) = issues(&app);
    let once = of(&found, "one-off");
    assert_eq!(once.len(), 1, "{found:#?}");
    assert!(once[0].title.starts_with("Burst Burst:4:"), "{}", once[0].title);
    assert!(once[0].ms.is_some() && once[0].total_ms.unwrap() > 20.0 && once[0].once == Some(60.0), "{:#?}", once[0]);
    assert!(once[0].severity <= Severity::Serious, "{:#?}", once[0]);
    let hitch = of(&found, "frame hitch");
    assert_eq!(hitch.len(), 1, "{found:#?}");
    assert!(hitch[0].evidence.iter().any(|line| line.starts_with("what worked in it: Burst Burst:4")), "{:?}", hitch[0].evidence);
    assert!(hot(&found).iter().all(|issue| !issue.title.contains("Burst Burst:4")), "{found:#?}");
}

/// The work of the code the issues cover, a frame of the reported phases: the game's when the harness's is apart
fn code_per_frame(app: &App) -> f64 {
    let p = app.profile();
    let (game, harness) = p.scope_work().unwrap();
    let frames = p.scope_frames(&p.scope()).unwrap();
    let apart = p.has_harness() && ree_profile_view::model::harness_apart(game, harness);
    (if apart { game } else { game + harness }) / frames
}

fn total_per_frame(app: &App) -> f64 {
    let profile = app.profile();
    let (game, harness) = profile.scope_work().unwrap();
    (game + harness) / profile.scope_frames(&profile.scope()).unwrap()
}

/// What the priced issues of the measured work add up to, a frame (host time, and work the run's clock did not charge,
/// aside)
fn priced_sum(found: &[Issue]) -> f64 {
    found.iter().filter(|issue| !issue.host_time && issue.kind != "work below floor" && issue.ms.is_some()).map(|issue| issue.counted_ms).sum()
}

/// The issues' kinds and severities, in the list's order (the priced, and the events with no rate)
fn ranking(found: &[Issue]) -> Vec<(&'static str, Severity)> {
    found.iter().filter(|issue| issue.kind != "work below floor" && (issue.ms.is_some() || issue.once.is_some())).map(|issue| (issue.kind, issue.severity)).collect()
}

/// The function issue whose title names `label` (`Steady Mod:4`)
fn function_issue<'a>(found: &'a [Issue], label: &str) -> Option<&'a Issue> {
    found.iter().find(|issue| issue.kind.starts_with("hot") && issue.title.contains(label))
}

fn steady_burst_module() -> String {
    let (steady, burst) = (work_bytes(0.96), work_bytes(19.2));
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local frame, _s, _b = 0, string.rep("a", {steady}), string.rep("a", {burst})

function M.Steady()
	return _m(_s, _p) and 1 or 0
end





function M.Burst()
	return _m(_b, _p) and 1 or 0
end

function M.Tick()
	frame += 1
	local s = M.Steady()
	if frame % 60 == 30 then
		s += M.Burst()
	end
	return s
end

return M
"#
    )
}

#[test]
fn the_same_code_profiled_for_two_seconds_and_ten_ranks_the_same() {
    // Steady's 1 ms every frame and Burst's 19 ms once a second: seen twice in 2 s and ten times in 10 s, the bursts have a
    // rate, so Burst's issue is priced a frame, the same either way, and so is the ranking with its severities
    let temp = Temp::new("planted-run-length");
    let source = steady_burst_module();
    let handler = harness("SteadyBurst", "sink += M.Tick()");
    let (short_out, long_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "two", "SteadyBurst", &source, &handler, "work,timeline", "2"));
        let h2 = scope.spawn(|| run_dir(&temp, "ten", "SteadyBurst", &source, &handler, "work,timeline", "10"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (short, long) = (open_out(&short_out), open_out(&long_out));
    let ((a, _), (b, _)) = (issues(&short), issues(&long));
    let priced = |found: &[Issue]| found.iter().filter(|issue| issue.ms.is_some() && issue.kind != "work below floor").map(|issue| (issue.kind, issue.severity, issue.title.split(": ").next().unwrap_or_default().to_string())).collect::<Vec<_>>();
    assert_eq!(priced(&a), priced(&b), "{a:#?}\n{b:#?}");
    let (burst_a, burst_b) = (function_issue(&a, "Burst SteadyBurst:12").unwrap(), function_issue(&b, "Burst SteadyBurst:12").unwrap());
    assert!((burst_a.ms.unwrap() / burst_b.ms.unwrap() - 1.0).abs() < 0.03, "{:?} against {:?}", burst_a.ms, burst_b.ms);
    assert!(burst_a.title.contains("active in 2 of 120 frames"), "{}", burst_a.title);
    let (steady_a, steady_b) = (function_issue(&a, "Steady SteadyBurst:4").unwrap(), function_issue(&b, "Steady SteadyBurst:4").unwrap());
    assert!((steady_a.ms.unwrap() / steady_b.ms.unwrap() - 1.0).abs() < 0.02, "{:?} against {:?}", steady_a.ms, steady_b.ms);
}

/// A harness for a module whose Work (0.59 ms of the work clock) runs in `fraction` of the frames, spread evenly. Tick
/// is told the fraction in twentieths, an integer, so exactly that many frames of every twenty run Work
fn fraction_harness(fraction: f64) -> String {
    let twentieths = (fraction * 20.0).round() as usize;
    harness("Fraction", &format!("sink += M.Tick({twentieths})"))
}

fn fraction_module() -> String {
    let work = work_bytes(0.59);
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local due, _w = 0, string.rep("a", {work})

function M.Work()
	return _m(_w, _p) and 1 or 0
end

function M.Tick(twentieths)
	due += twentieths
	if due >= 20 then
		due -= 20
		return M.Work()
	end
	return 0
end

return M
"#
    )
}

#[test]
fn work_on_more_frames_moves_its_price_and_severity_steadily() {
    // The full mean grows with duty, including the 5% case below the mean floor: its own .59 ms peak still lists it.
    let temp = Temp::new("planted-sweep");
    let source = fraction_module();
    let outs = parallel_map((1..=19).collect(), |twentieth| {
        let fraction = twentieth as f64 / 20.0;
        let out = run_dir(&temp, &format!("f{twentieth}"), "Fraction", &source, &fraction_harness(fraction), "work,timeline", "2");
        (twentieth, fraction, out)
    });
    let mut rows: Vec<(f64, f64, f64, Severity)> = Vec::new();
    for (twentieth, fraction, out) in outs {
        let app = open_out(&out);
        let (found, _) = issues(&app);
        let work = code_per_frame(&app);
        let profile = app.profile();
        let scope = profile.scope();
        let function = profile.funcs.iter().find(|f| f.name == "Work").unwrap();
        let calls: f64 = function.phases.iter().filter(|(phase, _)| scope.covers(**phase)).map(|(_, figures)| figures.calls).sum();
        // (a 2 s run reported over 120 frames: each twentieth of them is 6 frames)
        let frames_in_a_twentieth = 6.0;
        assert_eq!(calls, twentieth as f64 * frames_in_a_twentieth, "integer cadence must select exactly the planted fraction");
        assert_accounted(&app, &found);
        let issue = function_issue(&found, "Work Fraction:4");
        let issue = issue.unwrap_or_else(|| panic!("{fraction}: {found:#?}"));
        let price = issue.ms.unwrap();
        assert!(price <= work + 1e-9 && price >= 0.98 * work, "{fraction}: {price} of {work}");
        rows.push((fraction, work, price, issue.severity));
    }
    for pair in rows.windows(2) {
        let ((f0, w0, p0, s0), (f1, w1, p1, s1)) = (pair[0], pair[1]);
        assert!(p1 >= p0 && (p1 / p0) <= (w1 / w0) * 1.02, "{f0} to {f1}: {p0} ms a frame to {p1} as the work goes {w0} to {w1}: {rows:?}");
        assert!(s1 <= s0 && (s0 as i32) - (s1 as i32) <= 1, "{f0} to {f1}: {s0:?} to {s1:?}: {rows:?}");
    }
}

fn big_burst_module() -> String {
    let burst = work_bytes(76.8);
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local frame, _b = 0, string.rep("a", {burst})

function M.Steady()
	local s = 0
	for i = 1, 15000 do
		s += i % 7
	end
	return s
end

function M.Other()
	local s = 0
	for i = 1, 31000 do
		s += i % 5
	end
	return s
end

function M.Burst()
	return _m(_b, _p) and 1 or 0
end

function M.Tick()
	frame += 1
	local s = M.Steady() + M.Other()
	if frame == 60 then
		s += M.Burst()
	end
	return s
end

return M
"#
    )
}

#[test]
fn a_big_one_off_does_not_hide_a_steady_hot_function() {
    // Steady's 96 µs a frame beside Other's 200 µs, and Burst's 77 ms once, in frame 60: the one-off is its own issue, so
    // Steady's share is of the work a frame (a third), not of the run's (a tenth), and it is hot
    let temp = Temp::new("planted-big-burst");
    let (_, app) = run(&temp, "big", "BigBurst", &big_burst_module(), &harness("BigBurst", "sink += M.Tick()"), "work,timeline", "2");
    let (found, _) = issues(&app);
    let once = of(&found, "one-off");
    assert_eq!(once.len(), 1, "{found:#?}");
    assert_eq!(once[0].severity, Severity::Critical, "{:#?}", once[0]);
    assert!(once[0].title.starts_with("Burst BigBurst:20:"), "{}", once[0].title);
    let steady = function_issue(&found, "Steady BigBurst:4").unwrap_or_else(|| panic!("{found:#?}"));
    let ms = steady.ms.unwrap();
    assert!(ms > 0.08 && ms < 0.12, "{ms} ms a frame");
    assert!(hot(&found).iter().all(|issue| !issue.title.contains("Burst BigBurst:20")), "{found:#?}");
    assert!(priced_sum(&found) <= code_per_frame(&app) + 1e-9, "{found:#?}");
}

fn four_bursts_module() -> String {
    let burst = work_bytes(19.2);
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local frame, _b = 0, string.rep("a", {burst})

function M.A()
	return _m(_b, _p) and 1 or 0
end





function M.B()
	return _m(_b, _p) and 1 or 0
end





function M.C()
	return _m(_b, _p) and 1 or 0
end





function M.D()
	return _m(_b, _p) and 1 or 0
end





function M.Steady()
	local s = 0
	for i = 1, 15000 do
		s += i % 3
	end
	return s
end

function M.Tick()
	frame += 1
	local s = M.Steady()
	if frame == 60 then
		s += M.A() + M.B() + M.C() + M.D()
	end
	return s
end

return M
"#
    )
}

#[test]
fn a_one_off_spread_over_four_functions_names_each_and_is_never_steady_work() {
    // frame 60 runs a one-off in four functions, A, B, C and D (19 ms each): each is a one-off issue of its own, the
    // frame's hitch names all four with their shares, none is advised as steady work, and Steady, which runs every frame,
    // is hot
    let temp = Temp::new("planted-four");
    let (_, app) = run(&temp, "four", "Four", &four_bursts_module(), &harness("Four", "sink += M.Tick()"), "work,timeline", "2");
    let (found, _) = issues(&app);
    let functions = ["A Four:4", "B Four:12", "C Four:20", "D Four:28"];
    let once = of(&found, "one-off");
    assert_eq!(once.len(), 4, "{found:#?}");
    for function in functions {
        assert!(once.iter().any(|issue| issue.title.contains(function)), "{function}: {once:#?}");
    }
    let hitch = of(&found, "frame hitch");
    let named = hitch[0].evidence.iter().find(|line| line.starts_with("what worked in it: ")).unwrap_or_else(|| panic!("{:?}", hitch[0].evidence));
    for function in functions {
        assert!(named.contains(function), "{function}: {named}");
    }
    assert!(hot(&found).iter().all(|issue| functions.iter().all(|function| !issue.title.contains(function))), "{found:#?}");
    assert!(function_issue(&found, "Steady Four:36").is_some(), "{found:#?}");
}

/// A harness that reads `part` a frame through the module's Read
fn part_harness(name: &str) -> String {
    format!("return function(Context)\n\tlocal RunService = game:GetService(\"RunService\")\n\tlocal M = require(game.ReplicatedStorage.Game.{name})\n\tlocal part = Instance.new(\"Part\")\n\tpart.Anchored = true\n\tpart.Name = \"Probe\"\n\tpart.Parent = workspace\n\ttask.wait()\n\tContext.ProfilePhase(\"steady\")\n\tlocal sink = 0\n\tRunService.Heartbeat:Connect(function()\n\t\tsink += M.Read(part)\n\tend)\n\ttask.wait(Context.Seconds)\nend\n")
}

/// How many times a frame the repeat-read modules read a member
const READS_A_FRAME: usize = 2000;

/// A module whose Read reads `part.<member>` 2,000 times a frame (`many`), or once into a local, using it as `using`
/// says (`{v}` is the value). With `later`, the reads are in a function Read calls in a loop, so the repeats are in later
/// calls of the same function in the frame, not in one call
fn reads_module(member: &str, using: &str, many: bool, later: bool) -> String {
    if later {
        // `Value` reads the member (or is handed it); `Read` calls it, keeping the read in a local when it is not `many`
        let value = if many { format!("part.{member}") } else { "part".to_string() };
        let use_value = using.replace("{v}", &value);
        let keep_in_local = if many { String::new() } else { format!(" local value = part.{member}
") };
        let argument = if many { "part" } else { "value" };
        return format!("local M = {{}}
function M.Value(part)
 return {use_value}
end
function M.Read(part)
 local sum = 0
{keep_in_local} for _ = 1, {READS_A_FRAME} do sum += M.Value({argument}) end
 return sum
end
return M
");
    }
    if many {
        let read = using.replace("{v}", &format!("part.{member}"));
        format!("local M = {{}}

function M.Read(part)
	local s = 0
	for _ = 1, {READS_A_FRAME} do
		s += {read}
	end
	return s
end

return M
")
    } else {
        let read = using.replace("{v}", "value");
        format!("local M = {{}}

function M.Read(part)
	local s = 0
	local value = part.{member}
	for _ = 1, {READS_A_FRAME} do
		s += {read}
	end
	return s
end

return M
")
    }
}

#[test]
fn repeat_reads_are_priced_at_what_keeping_the_value_saves_for_every_kind_of_member() {
    // the same loop reading a part's member 2,000 times a frame, and once (the value kept in a local), for members a getter
    // serves (Position), plain ones (Size, Transparency, Anchored, CFrame, Color, Material) and the Instance API's own
    // (Name, Parent): the repeat read issue of the first prices its 1,999 repeats at what the second saves, the difference in
    // the game's work a frame, whichever path the read takes, and for repeats in one call and in later calls alike
    let temp = Temp::new("planted-repeat-price");
    let members = [
        ("Position", "{v}.X"),
        ("Orientation", "{v}.X"),
        ("Size", "{v}.X"),
        ("Transparency", "{v}"),
        ("Anchored", "(if {v} then 1 else 0)"),
        ("CFrame", "{v}.X"),
        ("Color", "{v}.R"),
        ("Material", "{v}.Value"),
        ("Name", "#{v}"),
        ("Parent", "(if {v} then 1 else 0)"),
    ];
    let jobs: Vec<(&str, &str, bool, bool)> = members
        .into_iter()
        .flat_map(|(member, using)| [false, true].into_iter().flat_map(move |later| [(member, using, later, true), (member, using, later, false)]))
        .collect();
    let mut outs = parallel_map(jobs, |(member, using, later, many)| {
        let (tag, name) = if many { ("many", "Many") } else { ("once", "Once") };
        run_dir(&temp, &format!("{member}-{later}-{tag}"), name, &reads_module(member, using, many, later), &part_harness(name), "work,timeline,boundary", "1")
    })
    .into_iter();
    let mut results = Vec::new();
    for (member, _) in members {
        for later in [false, true] {
            let many = open_out(&outs.next().unwrap());
            let once = open_out(&outs.next().unwrap());

            let (found, _) = issues(&many);
            let reads = of(&found, "repeat reads");
            assert_eq!(reads.len(), 1, "{member}, later={later}: {found:#?}");
            if later {
                assert!(reads[0].advice.contains("later calls"), "{:#?}", reads[0]);
            }
            let price_ms = reads[0].ms.unwrap();
            let saved_ms = code_per_frame(&many) - code_per_frame(&once);
            assert!(reads[0].upper_ms.is_some() && reads[0].cost_text().contains("recorded work"));
            assert!(saved_ms >= 0.0 && saved_ms <= reads[0].upper_ms.unwrap() + 1e-9);
            let subtotal: f64 = many.profile().boundary.as_ref().unwrap().repeat_reads.iter().map(|site| site.repeat_cost_ms.unwrap_or(0.0)).sum();
            assert!((price_ms - subtotal * many.profile().price_scale() / reads[0].reported_frames).abs() < 1e-12);
            results.push((member, later, price_ms, saved_ms));

            let (found, _) = issues(&once);
            assert!(of(&found, "repeat reads").is_empty(), "{member}: {found:#?}");
        }
    }
    // The subtotal stays close to the source edit; the explicit bounds above establish its precision.
    for (member, later, price_ms, saved_ms) in &results {
        assert!(*saved_ms > 0.0 && (price_ms / saved_ms - 1.0).abs() < 0.03, "{member}, later={later}: {price_ms} priced vs {saved_ms} saved: {results:?}");
    }
}

const BOARD: &str = "local M = {}\nlocal entries = {}\nfor i = 1, 3000 do\n\tentries[i] = { id = i, score = (i * 7919) % 1000 }\nend\n\nfunction M.Board()\n\tfor i, e in entries do\n\t\te.score = (e.score + i) % 1000\n\tend\n\ttable.sort(entries, function(a, b)\n\t\treturn a.score > b.score\n\tend)\n\treturn entries[1].id\nend\n\nreturn M\n";

#[test]
fn a_bounded_sort_cannot_claim_the_strongest_saving() {
    // The sort's recorded part exceeds Board's own work, but the complete edit has a whole-code bound, and a bound
    // never claims "nothing elsewhere can save as much".
    let temp = Temp::new("planted-board");
    let (_, app) = run(&temp, "board", "Board", BOARD, &harness("Board", "sink += M.Board()"), "work,timeline", "0.5");
    let (found, _) = issues(&app);
    let sorts = of(&found, "sort calls");
    assert_eq!(sorts.len(), 1, "{found:#?}");
    let board = function_issue(&found, "Board Board:7").unwrap_or_else(|| panic!("{found:#?}"));
    assert!(sorts[0].ms.unwrap() > board.ms.unwrap(), "{found:#?}");
    assert!(sorts[0].upper_ms.is_some());
    assert!(found.iter().all(|issue| !issue.advice.contains("nothing elsewhere")), "{found:#?}");
    assert!(priced_sum(&found) <= code_per_frame(&app) + 1e-9, "{found:#?}");
}

const FIFTH: &str = "local M = {}\nlocal frame = 0\n\nfunction M.Fifth()\n\tlocal s = 0\n\tfor i = 1, 92000 do\n\t\ts += i % 7\n\tend\n\treturn s\nend\n\nfunction M.Base()\n\tlocal s = 0\n\tfor i = 1, 19000 do\n\t\ts += i % 5\n\tend\n\treturn s\nend\n\nfunction M.Once()\n\tlocal s = 0\n\tfor i = 1, 70000 do\n\t\ts += i % 3\n\tend\n\treturn s\nend\n\nfunction M.Tick(once)\n\tframe += 1\n\tlocal s = M.Base()\n\tif frame % 5 == 0 then\n\t\ts += M.Fifth()\n\tend\n\tif once and frame == 62 then\n\t\ts += M.Once()\n\tend\n\treturn s\nend\n\nreturn M\n";

#[test]
fn an_unrelated_one_off_leaves_a_function_that_runs_every_fifth_frame_its_own_issue() {
    // Fifth's 0.59 ms every fifth frame beside Base's 120 us every frame, with and without Once's 0.45 ms in
    // frame 62. The one-off is Once's issue alone: Fifth keeps its issue and its price, and the priced issues add up to
    // the same work a frame either way
    let temp = Temp::new("planted-fifth");
    let h_without = harness("Fifth", "sink += M.Tick(false)");
    let h_with = harness("Fifth", "sink += M.Tick(true)");
    let (without_out, with_out) = std::thread::scope(|scope| {
        let a = scope.spawn(|| run_dir(&temp, "without", "Fifth", FIFTH, &h_without, "work,timeline", "2"));
        let b = scope.spawn(|| run_dir(&temp, "with", "Fifth", FIFTH, &h_with, "work,timeline", "2"));
        (a.join().unwrap(), b.join().unwrap())
    });
    let (without, with) = (open_out(&without_out), open_out(&with_out));
    let ((a, _), (b, _)) = (issues(&without), issues(&with));
    let (fifth_a, fifth_b) = (function_issue(&a, "Fifth Fifth:4").unwrap_or_else(|| panic!("{a:#?}")), function_issue(&b, "Fifth Fifth:4").unwrap_or_else(|| panic!("{b:#?}")));
    assert!((fifth_a.ms.unwrap() / fifth_b.ms.unwrap() - 1.0).abs() < 0.01, "{:?} against {:?}", fifth_a.ms, fifth_b.ms);
    assert_accounted(&without, &a);
    assert_accounted(&with, &b);
    let added = code_per_frame(&with) - code_per_frame(&without);
    let closing = |found: &[Issue]| found.iter().find(|issue| issue.kind == "work below floor").unwrap().counted_ms;
    let accounted_added = priced_sum(&b) + closing(&b) - priced_sum(&a) - closing(&a);
    assert!((accounted_added - added).abs() < 1e-8);
    let once = of(&b, "one-off");
    assert!(once.len() == 1 && once[0].title.starts_with("Once Fifth:20:"), "{b:#?}");
    assert!((once[0].ms.unwrap() * 120.0 - once[0].total_ms.unwrap()).abs() < 1e-8);
}

fn cadences_module() -> String {
    let (ai, ui, base) = (work_bytes(7.0), work_bytes(10.0), work_bytes(2.0));
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local frame, _a, _u, _b = 0, string.rep("a", {ai}), string.rep("a", {ui}), string.rep("a", {base})

function M.Ai()
	return _m(_a, _p) and 1 or 0
end





function M.Ui()
	return _m(_u, _p) and 1 or 0
end





function M.Base()
	return _m(_b, _p) and 1 or 0
end

function M.Tick()
	frame += 1
	local s = M.Base()
	if frame % 3 == 0 then
		s += M.Ai()
	end
	if frame % 7 == 0 then
		s += M.Ui()
	end
	return s
end

return M
"#
    )
}

#[test]
fn every_function_of_game_like_cadences_is_named_with_its_own_work() {
    // Ai's 7 ms every third frame, Ui's 10 ms every seventh and Base's 2 ms every frame: each has its own issue holding its own
    // work, and the frames where Ai and Ui meet overrun the frame, the hitch naming all three with their shares
    let temp = Temp::new("planted-cadences");
    let (_, app) = run(&temp, "cadences", "Cadence", &cadences_module(), &harness("Cadence", "sink += M.Tick()"), "work,timeline", "2");
    let (found, _) = issues(&app);
    let price = |label: &str| function_issue(&found, label).unwrap_or_else(|| panic!("{label}: {found:#?}")).ms.unwrap();
    let (ai, ui, base) = (price("Ai Cadence:4"), price("Ui Cadence:12"), price("Base Cadence:20"));
    assert!((ai / ui - (7.0 / 3.0) / (10.0 / 7.0)).abs() < 0.08, "Ai {ai}, Ui {ui}");
    assert!((ui / base - (10.0 / 7.0) / 2.0).abs() < 0.08, "Ui {ui}, Base {base}");
    assert!((priced_sum(&found) / code_per_frame(&app) - 1.0).abs() < 0.01, "{found:#?}");
    let hitch = of(&found, "frame hitch");
    assert!(!hitch.is_empty(), "{found:#?}");
    let named = hitch[0].evidence.iter().find(|line| line.starts_with("what worked in it: ")).unwrap();
    for function in ["Ai Cadence:4", "Ui Cadence:12", "Base Cadence:20"] {
        assert!(named.contains(function), "{function}: {named}");
    }
}

fn base_burst_module() -> String {
    let (base, extra) = (work_bytes(1.18), work_bytes(2.95));
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local _b, _e = string.rep("a", {base}), string.rep("a", {extra})
function M.Work(extra)
	local s = _m(_b, _p) and 1 or 0
	if extra then
		s += _m(_e, _p) and 1 or 0
	end
	return s
end

local frame = 0
function M.Tick(bursts)
	frame += 1
	return M.Work(bursts and frame % 120 == 30)
end

return M
"#
    )
}

#[test]
fn a_burst_on_top_of_steady_work_claims_what_doing_it_ahead_saves() {
    // Work's full mean includes its 1.18 ms base and each observed 2.95 ms pulse. Removing the pulses changes that
    // mean by the actual A/B saving; neither run has a separate burst price.
    let temp = Temp::new("planted-base-burst");
    let source = base_burst_module();
    let h_with = harness("BaseBurst", "sink += M.Tick(true)");
    let h_without = harness("BaseBurst", "sink += M.Tick(false)");
    let (with_out, without_out) = std::thread::scope(|scope| {
        let a = scope.spawn(|| run_dir(&temp, "with", "BaseBurst", &source, &h_with, "work,timeline", "4"));
        let b = scope.spawn(|| run_dir(&temp, "without", "BaseBurst", &source, &h_without, "work,timeline", "4"));
        (a.join().unwrap(), b.join().unwrap())
    });
    let (with, without) = (open_out(&with_out), open_out(&without_out));
    let (found, _) = issues(&with);
    let work = function_issue(&found, "Work BaseBurst:3").unwrap_or_else(|| panic!("{found:#?}"));
    let (base, _) = issues(&without);
    let base = function_issue(&base, "Work BaseBurst:3").unwrap();
    let saved = code_per_frame(&with) - code_per_frame(&without);
    assert!(((work.ms.unwrap() - base.ms.unwrap()) / saved - 1.0).abs() < 0.03);
    assert!(work.title.contains("active in 240 of 240 frames"));
}

fn fifteen_module() -> String {
    let (rare, base) = (work_bytes(1.0), work_bytes(0.333));
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local due, _r, _b = 0, string.rep("a", {rare}), string.rep("a", {base})

function M.Rare()
	return _m(_r, _p) and 1 or 0
end





function M.Base()
	return _m(_b, _p) and 1 or 0
end

function M.Tick()
	local s = M.Base()
	due += 0.15
	if due >= 1 then
		due -= 1
		s += M.Rare()
	end
	return s
end

return M
"#
    )
}

#[test]
fn work_on_fifteen_percent_of_the_frames_is_never_listed_nowhere() {
    // Rare's 1 ms on 15% of the frames (150 µs a frame, 31% of the game's work) beside Base's 333 µs every frame: Rare's
    // issue holds all its work in the recurring part, and the issues hold the game's work a frame
    let temp = Temp::new("planted-fifteen");
    let (_, app) = run(&temp, "fifteen", "Fifteen", &fifteen_module(), &harness("Fifteen", "sink += M.Tick()"), "work,timeline", "2");
    let (found, _) = issues(&app);
    let rare = function_issue(&found, "Rare Fifteen:4").unwrap_or_else(|| panic!("{found:#?}"));
    assert!((rare.ms.unwrap() - 0.15).abs() < 0.02, "{rare:#?}");
    assert!(rare.active_frames >= 17 && rare.active_frames <= 18);
    assert!((priced_sum(&found) / code_per_frame(&app) - 1.0).abs() < 0.02, "{} of {}: {found:#?}", priced_sum(&found), code_per_frame(&app));
}

fn tenth_module(period: usize) -> String {
    let big = work_bytes(12.0);
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local frame, _b = 0, string.rep("a", {big})
local entries = {{}}
for i = 1, 300 do
	entries[i] = {{ id = i, score = (i * 7919) % 1000 }}
end

function M.Big()
	return _m(_b, _p) and 1 or 0
end

function M.Board()
	for i, e in entries do
		e.score = (e.score + i) % 1000
	end
	table.sort(entries, function(a, b)
		return a.score > b.score
	end)
	return entries[1].id
end

function M.Tick()
	frame += 1
	local s = M.Board()
	if frame % {period} == 5 then
		s += M.Big()
	end
	return s
end

return M
"#
    )
}

#[test]
fn a_big_burst_every_tenth_frame_ranks_by_its_price() {
    // Big's 12 ms every 8th and every 10th frame (1.2 to 1.5 ms a frame: serious by its rate, though each burst fits in its
    // frame) against a sort of 300 entries every frame: Big's issue ranks above the sort by its price, and its rate is the
    // same at 2 and 10 seconds. Everything is accounted for
    let temp = Temp::new("planted-tenth");
    let handler = harness("Tenth", "sink += M.Tick()");
    let jobs = vec![(8, 2), (8, 10), (10, 2), (10, 10)];
    let mut outs = parallel_map(jobs, |(period, seconds)| {
        let source = tenth_module(period);
        run_dir(&temp, &format!("p{period}-s{seconds}"), "Tenth", &source, &handler, "work,timeline", &seconds.to_string())
    })
    .into_iter();
    for _period in [8, 10] {
        let mut rates = Vec::new();
        for _seconds in [2, 10] {
            let app = open_out(&outs.next().unwrap());
            let (found, _) = issues(&app);
            assert_accounted(&app, &found);
            let big = function_issue(&found, "Big Tenth:8").unwrap_or_else(|| panic!("{found:#?}"));
            assert_eq!(big.severity, Severity::Serious);
            let sorts = of(&found, "sort calls");
            assert_eq!(sorts.len(), 1, "{found:#?}");
            let at = |issue: &Issue| found.iter().position(|other| other.title == issue.title).unwrap();
            assert!(at(big) < at(sorts[0]) && big.ms > sorts[0].ms, "{found:#?}");
            rates.push(big.ms.unwrap());
        }
        assert!((rates[0] / rates[1] - 1.0).abs() < 0.03, "{rates:?}");
    }
}

fn once_beside_module() -> String {
    let (steady, once) = (work_bytes(3.0), work_bytes(19.0));
    format!(
        r#"local M, _m, _p = {{}}, string.match, "{WORK_PATTERN}"
local frame, _s, _o = 0, string.rep("a", {steady}), string.rep("a", {once})

function M.Steady()
	return _m(_s, _p) and 1 or 0
end





function M.Once()
	return _m(_o, _p) and 1 or 0
end

function M.Tick()
	frame += 1
	local s = M.Steady()
	if frame == 60 then
		s += M.Once()
	end
	return s
end

return M
"#
    )
}

#[test]
fn a_one_off_beside_a_hot_line_ranks_the_same_at_two_seconds_and_ten() {
    // Once keeps its full 19 ms size and honest mean rate. Its own hitch makes it warning at 2, 5 and 10 seconds,
    // beside Steady's critical 3 ms/frame.
    let temp = Temp::new("planted-once-beside");
    let source = once_beside_module();
    let handler = harness("Beside", "sink += M.Tick()");
    let outs = parallel_map(vec![2, 5, 10], |seconds| {
        run_dir(&temp, &seconds.to_string(), "Beside", &source, &handler, "work,timeline", &seconds.to_string())
    });
    let mut ranks = Vec::new();
    for out in outs {
        let app = open_out(&out);
        let (found, _) = issues(&app);
        assert_accounted(&app, &found);
        let once = of(&found, "one-off");
        assert_eq!(once.len(), 1, "{found:#?}");
        assert!(once[0].once == Some(60.0) && (once[0].total_ms.unwrap() - 19.0).abs() < 0.1, "{found:#?}");
        assert_eq!(once[0].severity, Severity::Warning);
        assert_eq!(once[0].severity, Severity::of_hitch(once[0].total_ms.unwrap(), app.profile().budget_ms()));
        ranks.push(ranking(&found).into_iter().map(|(kind, severity)| (kind.to_string(), severity)).collect::<Vec<_>>());
    }
    assert!(ranks.iter().all(|rank| rank == &ranks[0]), "{ranks:?}");
}

/// The listing floor in ms a frame: 0.25% of a 60 fps frame
const LIST_FLOOR_MS: f64 = 1000.0 / 60.0 * 0.0025;

/// The closing row plus the union of fixes and events must account for every measured millisecond, every listed fix must
/// reach the floor, and its severity is the one its own saving earns
fn assert_accounted(app: &App, found: &[Issue]) {
    let measured = code_per_frame(app);
    let union: f64 = found.iter().map(|issue| issue.counted_ms).sum();
    assert!((union - measured).abs() < 1e-8, "{union} accounted of {measured}: {found:#?}");

    let closing_rows: Vec<&Issue> = found.iter().filter(|issue| issue.kind == "work below floor").collect();
    assert_eq!(closing_rows.len(), 1);
    assert!(std::ptr::eq(closing_rows[0], found.last().unwrap()), "the closing row is last");

    for issue in found.iter().filter(|issue| issue.ms.is_some() && issue.kind != "work below floor") {
        let price_ms = issue.ms.unwrap();
        assert!(price_ms >= LIST_FLOOR_MS - 1e-9 || issue.frame_ms.unwrap_or(0.0) >= app.profile().budget_ms() * 0.025, "{issue:#?}");
        let severity = Severity::of(price_ms, app.profile().budget_ms()).min(Severity::of_hitch(issue.over_budget_ms, app.profile().budget_ms()));
        assert_eq!(issue.severity, severity);
    }
}

/// A module whose Pulse does `ms` milliseconds of work on the work clock, and whose Tick runs Pulse in the frames
/// `active` says (given the frame number)
fn pulse_module(ms: f64) -> String {
    let bytes = work_bytes(ms);
    format!(
        "local M, _m, _p, _s = {{}}, string.match, \"{WORK_PATTERN}\", string.rep(\"a\", {bytes})\n\
function M.Pulse()\n return _m(_s, _p) and 1 or 0\nend\n\
local frame = 0\n\
function M.Tick(active)\n frame += 1\n if active(frame) then return M.Pulse() end\n return 0\nend\n\
return M\n"
    )
}

/// Pulse steps that cost 1 ms of the work clock (used by `steps_for_ms` for small Luau-loop events)
const PULSE_STEPS_FOR_ONE_MS: usize = 156_250;

#[test]
fn sustained_stretches_keep_their_full_rate_at_every_run_length() {
    // Pulse works 1 ms in each frame of a stretch at the start of the run, 10% to 50% of its frames, at three run lengths.
    // The stretch is no one-off: it keeps its full mean rate (the fraction of 1 ms), the same at every length
    let temp = Temp::new("planted-stretches");
    let frames_a_second = 60;
    let module = pulse_module(1.0);
    let fractions = [0.10, 0.20, 0.33, 0.50];
    let lengths = [2, 5, 10];
    let jobs: Vec<(f64, usize)> = fractions.into_iter().flat_map(|f| lengths.into_iter().map(move |s| (f, s))).collect();
    let mut outs = parallel_map(jobs, |(fraction, seconds)| {
        let frames = seconds * frames_a_second;
        let active_frames = (frames as f64 * fraction).round() as usize;
        let body = format!("sink += M.Tick(function(frame) return frame <= {active_frames} end)");
        let case = format!("f{active_frames}-s{seconds}");
        run_dir(&temp, &case, "Stretch", &module, &harness("Stretch", &body), "work,timeline", &seconds.to_string())
    })
    .into_iter();
    for fraction in fractions {
        let mut rates = Vec::new();
        for seconds in lengths {
            let app = open_out(&outs.next().unwrap());
            let (found, _) = issues(&app);
            assert_accounted(&app, &found);
            assert!(of(&found, "one-off").is_empty(), "{fraction} at {seconds}: {found:#?}");

            let pulse = function_issue(&found, "Pulse Stretch:2").unwrap_or_else(|| panic!("{found:#?}"));
            let rate_ms = pulse.ms.unwrap();
            assert!((rate_ms - fraction).abs() < 0.012, "{fraction} at {seconds}: {rate_ms} ms a frame");
            rates.push(rate_ms);
        }
        assert!(rates.iter().all(|rate| (rate / rates[0] - 1.0).abs() < 0.04), "{rates:?}");
    }
}

#[test]
fn contiguous_and_spaced_fifteen_percent_and_four_chunks_have_the_same_price() {
    // The same work arranged two ways costs the same: 15% of the frames spread over the run or in one stretch, and four
    // chunks of about 4.75 ms on alternate frames or on consecutive ones. Neither arrangement is a one-off
    let temp = Temp::new("planted-arrangements");
    let spread_fifteen_percent = "frame % 20 < 3";
    let stretch_fifteen_percent = "frame <= 18";
    let alternate_chunks = "frame == 30 or frame == 32 or frame == 34 or frame == 36";
    let consecutive_chunks = "frame >= 30 and frame <= 33";
    // (the price a frame in ms: 0.59 ms on 18 of 120 frames; four 4.75 ms chunks in 120 frames)
    let arrangements = [
        ("Fifteen", 0.59, [spread_fifteen_percent, stretch_fifteen_percent], 0.0885),
        ("Chunks", 4.75, [alternate_chunks, consecutive_chunks], 19.0 / 120.0),
    ];
    let jobs: Vec<(&str, f64, usize, &str)> = arrangements
        .iter()
        .flat_map(|&(name, ms, conditions, _)| conditions.into_iter().enumerate().map(move |(i, c)| (name, ms, i, c)))
        .collect();
    let mut outs = parallel_map(jobs, |(name, ms, index, condition)| {
        let body = format!("sink += M.Tick(function(frame) return {condition} end)");
        run_dir(&temp, &format!("{name}-{index}"), name, &pulse_module(ms), &harness(name, &body), "work,timeline", "2")
    })
    .into_iter();
    for (name, _, conditions, expected_ms) in arrangements {
        let mut rates = Vec::new();
        for _ in conditions {
            let app = open_out(&outs.next().unwrap());
            let (found, _) = issues(&app);
            assert_accounted(&app, &found);
            assert!(of(&found, "one-off").is_empty(), "{found:#?}");

            let pulse = function_issue(&found, &format!("Pulse {name}:2")).unwrap_or_else(|| panic!("{found:#?}"));
            assert!((pulse.ms.unwrap() / expected_ms - 1.0).abs() < 0.025, "{pulse:#?}");
            rates.push((pulse.ms.unwrap(), pulse.severity));
        }
        let (first, second) = (rates[0], rates[1]);
        assert!((first.0 / second.0 - 1.0).abs() < 0.01 && first.1 == second.1, "{rates:?}");
    }
}

#[test]
fn six_equal_functions_are_all_critical_and_small_transfers_conserve_the_total() {
    // Six functions of about 2.49 ms each are all critical and all listed. Two functions with 0.1 ms moved from one to the
    // other have the same total work; the listed fixes and the closing row between them account for all of it
    let temp = Temp::new("planted-floor");
    let six_bytes = work_bytes(2.49);
    let mut six_module = format!("local M, _m, _p, _w = {{}}, string.match, \"{WORK_PATTERN}\", string.rep(\"a\", {six_bytes})\n");
    let mut six_body = "sink += 0".to_string();
    for index in 0..6 {
        six_module.push_str(&format!("function M.Work{index}()\n return _m(_w, _p) and 1 or 0\nend\n"));
        six_body.push_str(&format!(" + M.Work{index}()"));
    }
    six_module.push_str("return M\n");
    let loop_case = |steps_by_function: &[usize]| {
        let mut module = "local M = {}\n".to_string();
        let mut body = "sink += 0".to_string();
        for (index, steps) in steps_by_function.iter().enumerate() {
            module.push_str(&format!("function M.Work{index}()\n local sum = 0\n for i = 1, {steps} do sum += i % 7 end\n return sum\nend\n"));
            body.push_str(&format!(" + M.Work{index}()"));
        }
        module.push_str("return M\n");
        (module, body)
    };
    let (before_mod, before_body) = loop_case(&[21_719, 6_719]);
    let (after_mod, after_body) = loop_case(&[6_094, 22_344]);
    let jobs = vec![("six", six_module, six_body), ("before", before_mod, before_body), ("after", after_mod, after_body)];
    let results = parallel_map(jobs, |(case, module, body)| {
        (case, run_dir(&temp, case, "Floor", &module, &harness("Floor", &body), "work,timeline", "2"))
    });
    for (case, out) in results {
        let app = open_out(&out);
        let (found, _) = issues(&app);
        assert_accounted(&app, &found);
        if case == "six" {
            let hot = hot(&found);
            assert_eq!(hot.len(), 6, "{found:#?}");
            assert!(hot.iter().all(|issue| issue.severity == Severity::Critical && (issue.ms.unwrap() - 2.49).abs() < 0.02));
        } else {
            let listed = priced_sum(&found);
            assert!((listed - 0.18).abs() < 0.1, "{listed}: {found:#?}");
        }
    }
}

#[test]
fn another_functions_hitch_never_changes_a_small_functions_severity() {
    // Small does about 80 us a frame (info). Once does 30 ms in frame 60 in one of the runs: its hitch must not raise
    // Small's severity, and Small's price is the same with and without it
    let temp = Temp::new("planted-severity");
    let small_steps = 12_438;
    let once_bytes = work_bytes(30.0);
    let module = format!(
        "local M, _m, _p, _o = {{}}, string.match, \"{WORK_PATTERN}\", string.rep(\"a\", {once_bytes})\n\
function M.Small()\n local sum = 0\n for i = 1, {small_steps} do sum += i % 7 end\n return sum\nend\n\
function M.Once()\n return _m(_o, _p) and 1 or 0\nend\n\
return M\n"
    );
    let outs = parallel_map(vec![false, true], |hitch| {
        let body = format!("sink += M.Small(); frame = (frame or 0) + 1; if {hitch} and frame == 60 then sink += M.Once() end");
        run_dir(&temp, &hitch.to_string(), "Isolation", &module, &harness("Isolation", &body), "work,timeline", "2")
    });
    let mut rates = Vec::new();
    for out in outs {
        let app = open_out(&out);
        let (found, _) = issues(&app);
        assert_accounted(&app, &found);

        let small = function_issue(&found, "Small Isolation:2").unwrap_or_else(|| panic!("{found:#?}"));
        assert_eq!(small.severity, Severity::Info);
        rates.push(small.ms.unwrap());
    }
    assert!((rates[0] / rates[1] - 1.0).abs() < 0.01, "{rates:?}");
}

#[test]
fn fifteen_hundred_functions_over_six_hundred_frames_keep_a_small_profile() {
    // 1,500 small functions that run every frame for 10 s (600 frames) are below the floor, so profile.json keeps no frame
    // history for them: it is at most half as big again as it would be without any, and the report page holds none
    let temp = Temp::new("planted-size");
    let function_count = 1500;
    // Keep ~3.2 us of work per function per frame (~4.8 ms/frame across 1,500 functions, above 25% of a 60 fps frame
    // and below the 50% / 8.3 ms heavy-frame line) using C match work so host time stays small.
    let work_size = work_bytes(0.0032);
    let mut source = format!("local M, _m, _p, _s = {{}}, string.match, \"{WORK_PATTERN}\", string.rep(\"a\", {work_size})\n");
    for index in 1..=function_count {
        source.push_str(&format!("function M.Work{index}()\n return _m(_s, _p) and 1 or 0\nend\n"));
    }
    source.push_str("function M.Tick()\n local sum = 0\n");
    for index in 1..=function_count {
        source.push_str(&format!(" sum += M.Work{index}()\n"));
    }
    source.push_str(" return sum\nend\nreturn M\n");

    let (out, app) = run(&temp, "many", "Many", &source, &harness("Many", "sink += M.Tick()"), "work,timeline", "10");
    let text = std::fs::read(out.join("profile.json")).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&text).unwrap();
    let history_bytes = json["timeline"]["node_frames"].to_string().len() + json["timeline"]["function_stats"].to_string().len();
    json["timeline"].as_object_mut().unwrap().remove("node_frames");
    json["timeline"].as_object_mut().unwrap().remove("function_stats");
    let bytes_without_history = json.to_string().len();
    assert!(text.len() as f64 <= bytes_without_history as f64 * 1.5, "{} bytes, {bytes_without_history} without frame history, {history_bytes} history", text.len());
    assert!(history_bytes < 100_000, "{history_bytes} bytes of frame history");

    let (found, _) = issues(&app);
    assert_accounted(&app, &found);
    let page = std::fs::read_to_string(out.join("report.html")).unwrap();
    assert!(!page.contains("\"function_stats\":"));
    assert!(!page.contains("\"node_frames\":"));
}

/// A harness like [`harness`] whose handler body can count frames in `frame`
fn counting_harness(require: &str, body: &str) -> String {
    harness(require, body).replace("local sink = 0", "local sink, frame = 0, 0")
}

/// The loop steps of `Pulse`-style work that cost `ms` of the work clock
fn steps_for_ms(ms: f64) -> usize {
    (ms * PULSE_STEPS_FOR_ONE_MS as f64).round() as usize
}

/// How far a measured price may sit from the one planted: the work clock's own rounding, in ms
const PRICE_TOLERANCE_MS: f64 = 0.002;

/// How far an issue's price may sit from what the same code saves when it is rewritten and profiled again (ms a frame)
const A_B_TOLERANCE_MS: f64 = 1e-6;

/// The frames the run lasts, at the frame rate the harness runs with
const FRAMES_A_SECOND: f64 = 60.0;

#[test]
fn review_events_survive_120_fps_and_a_phase_mark_inside_the_frame() {
    let temp = Temp::new("planted-review-events");

    // A 0.3 ms event in frame 30 of a 120 fps run, which is 240 frames in two seconds
    let small_event_ms = 0.3;
    let event_module = r#"local M = {}
function M.Small()
    local sum = 0
    for k = 1, STEPS do sum += k % 7 end
    return sum
end
return M
"#
    .replace("STEPS", &steps_for_ms(small_event_ms).to_string());
    let handler1 = counting_harness("Event", "frame += 1; if frame == 30 then sink += M.Small() end");
    let options1 = RunOptions { kinds: "work,timeline", seconds: "2", extra_arguments: &["--fps", "120"] };

    // The same work split by a phase mark inside frame 120: the two halves are parts of one frame and one event
    let split_module = r#"local M = {}
function M.Split(ctx)
    local sum = 0
    for k = 1, STEPS do sum += k % 7 end
    ctx.ProfilePhase("tail")
    for k = 1, STEPS do sum += k % 7 end
    return sum
end
return M
"#
    .replace("STEPS", &steps_for_ms(small_event_ms).to_string());
    let handler2 = counting_harness("Split", "frame += 1; if frame == 120 then sink += M.Split(Context) end");
    let options2 = RunOptions { kinds: "work,timeline", seconds: "10", extra_arguments: &["--profile-phase", "all"] };
    let (out1, out2) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir_with(&temp, "fps120", "Event", &event_module, &handler1, &options1));
        let h2 = scope.spawn(|| run_dir_with(&temp, "split", "Split", &split_module, &handler2, &options2));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (app1, app2) = (open_out(&out1), open_out(&out2));

    let (found, _) = issues(&app1);
    let once = of(&found, "one-off")[0];
    assert_eq!(once.once, Some(30.0));
    assert!((once.total_ms.unwrap() - 0.301).abs() < PRICE_TOLERANCE_MS, "{once:#?}");
    let frames_at_120_fps = 240.0;
    assert!((once.ms.unwrap() - once.total_ms.unwrap() / frames_at_120_fps).abs() < 1e-8);
    assert_accounted(&app1, &found);

    let (found, _) = issues(&app2);
    let once = of(&found, "one-off").into_iter().find(|issue| issue.title.starts_with("Split Split:2:")).unwrap();
    assert_eq!(once.once, Some(120.0));
    assert!((once.total_ms.unwrap() - 0.605).abs() < 0.003, "{once:#?}");
    assert!(once.ms.unwrap() < PRICE_TOLERANCE_MS);
    assert_accounted(&app2, &found);
}

#[test]
fn two_real_computations_are_priced_whole_without_subtracting_the_smaller() {
    let temp = Temp::new("planted-review-two");
    let spike_module = format!(
        "local M, _m, _p = {{}}, string.match, \"{WORK_PATTERN}\"\n\
function M.Spike(s)\n    return _m(s, _p) and 1 or 0\nend\n\
return M\n"
    );
    let (first_frame, second_frame) = (30, 80);
    let second_ms = 19.0;
    let cases = [("equal", 19.0), ("unequal", 25.0)];
    // (two equal computations, then a larger one before a smaller one: each is priced whole)
    let outs = parallel_map(cases.to_vec(), |(name, first_ms)| {
        let b1 = work_bytes(first_ms);
        let b2 = work_bytes(second_ms);
        let handler = counting_harness("Spike", &format!("frame += 1; if frame == {first_frame} then sink += M.Spike(_s1) elseif frame == {second_frame} then sink += M.Spike(_s2) end"))
            .replace("local sink, frame = 0, 0", &format!("local sink, frame, _s1, _s2 = 0, 0, string.rep(\"a\", {b1}), string.rep(\"a\", {b2})"));
        (first_ms, run_dir(&temp, name, "Spike", &spike_module, &handler, "work,timeline", "2"))
    });
    for (first_ms, out) in outs {
        let app = open_out(&out);
        let (found, _) = issues(&app);
        let issue = function_issue(&found, "Spike Spike:2").unwrap();
        assert_eq!(issue.active_frames, 2);
        assert!(issue.once.is_none());
        let frames = 2.0 * FRAMES_A_SECOND;
        assert!((issue.ms.unwrap() - (first_ms + second_ms) / frames).abs() < PRICE_TOLERANCE_MS, "{issue:#?}");
        assert!((issue.frame_ms.unwrap() - first_ms).abs() < 0.1);
        assert_accounted(&app, &found);
    }
}

/// A module whose Read calls `string.byte(s)` or similar `CALL` ten thousand times
fn builtin_call_module(call: &str) -> String {
    r#"local M = {}
function M.Read(s)
    local sum = 0
    for i = 1, 10000 do sum += CALL end
    return sum
end
return M
"#
    .replace("CALL", call)
}

/// The VM's replacement fields are consumed from actual profiles, with source setup and prototype code paid too.
#[test]
fn a_real_builtin_fix_bounds_its_unseen_source_setup() {
    let temp = Temp::new("planted-builtin-net");
    let handler = harness("Byte", "sink += M.Read(\"abcdef\")");
    let declined_mod = builtin_call_module("string.byte(s)");
    let fixed_mod = builtin_call_module("string.byte(s, 1)");
    let (before_out, after_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "declined", "Byte", &declined_mod, &handler, "work,caches", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "fixed", "Byte", &fixed_mod, &handler, "work,caches", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (before, after) = (open_out(&before_out), open_out(&after_out));
    let (found, _) = issues(&before);
    let declines = of(&found, "builtin declines");
    assert_eq!(declines.len(), 1, "compiled copies are one source edit: {found:#?}");
    let price = declines[0].ms.unwrap_or_else(|| panic!("{:#?}", declines[0]));
    // Added prototype code is charged at its caller's line, which this harness owns.
    let saved = total_per_frame(&before) - total_per_frame(&after);
    assert!(saved > 0.04);
    assert!(declines[0].at_most && declines[0].cost_text().starts_with("at most"));
    assert!(saved <= price + 1e-9, "bound {price} against A/B {saved}");
    assert_accounted(&before, &found);
    let builtins = before.profile().raw.builtins.as_ref().unwrap();
    let string_byte = builtins.iter().find(|builtin| builtin.name == "string.byte").unwrap();
    assert!(string_byte.sites.iter().all(|site| site.nodes.is_some() && site.fast_equivalent_us.is_some()));
    println!("real string.byte fix: {price:.12} ms/frame; A/B saves {saved:.12}");
}

#[test]
fn a_real_substring_fix_bounds_its_added_argument_instructions() {
    let temp = Temp::new("planted-substring-net");
    // (the length of the string changes what the call costs, not what the added argument does)
    let declined_call = builtin_call_module("#string.sub(s, 2)");
    let fixed_call = builtin_call_module("#string.sub(s, 2, #s)");
    let jobs: Vec<(usize, bool)> = [6, 1000].into_iter().flat_map(|len| [(len, false), (len, true)]).collect();
    let mut outs = parallel_map(jobs, |(length, fixed)| {
        let handler = harness("Sub", &format!("sink += M.Read(string.rep(\"a\", {length}))"));
        let (tag, source) = if fixed { ("fixed", &fixed_call) } else { ("declined", &declined_call) };
        run_dir(&temp, &format!("{tag}-{length}"), "Sub", source, &handler, "work,caches", "1")
    })
    .into_iter();
    for length in [6, 1000] {
        let before = open_out(&outs.next().unwrap());
        let after = open_out(&outs.next().unwrap());
        let (found, _) = issues(&before);
        let declines = of(&found, "builtin declines");
        assert_eq!(declines.len(), 1, "{found:#?}");
        let price = declines[0].ms.unwrap_or_else(|| panic!("{:#?}", declines[0]));
        // The added argument instructions also cost cold work at the harness's call site.
        let saved = total_per_frame(&before) - total_per_frame(&after);
        let builtin = before.profile().raw.builtins.as_ref().unwrap().iter().find(|builtin| builtin.name == "string.sub").unwrap();
        let expected = (builtin.decline_charged_us.unwrap() - builtin.fast_equivalent_us.unwrap()) / 1000.0 / declines[0].reported_frames;
        assert_same_ms(price, expected, "operand-preserving site bound");
        assert!(saved > 0.0);
        assert!(declines[0].at_most && saved <= price + 1e-9, "bound {price} against A/B {saved}");
        assert_accounted(&before, &found);
        println!("real string.sub fix, {length} characters: {price:.12} ms/frame; A/B saves {saved:.12}");
    }
}

#[test]
fn a_real_presized_constructor_is_priced_after_its_replacement_charge() {
    let temp = Temp::new("planted-rehash-net");
    let build_module = |constructor: &str| {
        r#"local M = {}
function M.Build()
    local result
    for i = 1, TABLES do
        local t = CONSTRUCTOR
        t.x = i
        result = t
    end
    return result.x
end
return M
"#
        .replace("TABLES", &TABLES_BUILT.to_string())
        .replace("CONSTRUCTOR", constructor)
    };
    const TABLES_BUILT: usize = 2000;
    // (the `t.x = i` write is on line 6; a table of four keys must grow to take it, a table built with x does not)
    let handler = harness("Cache", "sink += M.Build()");
    let before_mod = build_module("{a = 1, b = 1, c = 1, d = 1}");
    let after_mod = build_module("{a = 1, b = 1, c = 1, d = 1, x = 0}");
    let (before_out, after_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "growing", "Cache", &before_mod, &handler, "work,caches", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "presized", "Cache", &after_mod, &handler, "work,caches", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (before, after) = (open_out(&before_out), open_out(&after_out));
    let (found, _) = issues(&before);
    let rehash = of(&found, "cache misses").into_iter().find(|issue| issue.title.contains("Cache:6")).unwrap_or_else(|| panic!("{found:#?}"));
    let price = rehash.ms.unwrap();
    let saved = code_per_frame(&before) - code_per_frame(&after);
    assert!((price - saved).abs() < A_B_TOLERANCE_MS, "net site {price} ms/frame against A/B {saved} ms/frame");
    assert_accounted(&before, &found);
    println!("real presized constructor: {price:.12} ms/frame; A/B saves {saved:.12}; {:.6} ns/table", price * 1e6 / TABLES_BUILT as f64);
}

#[test]
fn a_real_substring_rewrite_charges_both_paths_for_their_result_strings() {
    let temp = Temp::new("planted-builtin-zero");
    let source = r#"local M = {}
function M.Read(s)
    local sum = 0
    for i = 1, 6000 do sum += #string.sub(s, i) end
    return sum
end
return M
"#;
    let fixed = source.replace("string.sub(s, i)", "string.sub(s, i, #s)");
    let handler = counting_harness("Sub", "frame += 1; sink += M.Read(string.rep(\"abcdef\", 200) .. frame)");
    let (app_out, after_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "substring", "Sub", source, &handler, "work,caches", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "fixed", "Sub", &fixed, &handler, "work,caches", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (app, after) = (open_out(&app_out), open_out(&after_out));
    let (found, _) = issues(&app);
    let declines = of(&found, "builtin declines");
    assert_eq!(declines.len(), 1, "{found:#?}");
    let saved = code_per_frame(&app) - code_per_frame(&after);
    let price = declines[0].ms.unwrap();
    assert!(saved > 0.0 && saved <= price + SAME_MS, "bound {price} against A/B {saved}");
    assert!(declines[0].at_most && !declines[0].no_saving);
    println!("substring allocation parity: bound {price:.12}, A/B {saved:.12} ms/frame");
    assert_accounted(&app, &found);
}

#[test]
fn a_real_append_site_part_retains_the_fast_paths_growth_charges() {
    let temp = Temp::new("planted-append-retained");
    let fill_module = |call: &str| {
        r#"local M = {}
function M.Fill()
    local values = {}
    for i = 1, 12000 do CALL end
    return #values
end
return M
"#
        .replace("CALL", call)
    };
    let handler = harness("Append", "sink += M.Fill()");
    let declined_mod = fill_module("table.insert(values, i, i)");
    let fast_mod = fill_module("table.insert(values, i)");
    // (inserting at a position that is not the end is declined; at the end it is the fast path, which keeps rehashing)
    let (declined_out, fast_out) = std::thread::scope(|scope| {
        let h1 = scope.spawn(|| run_dir(&temp, "declined", "Append", &declined_mod, &handler, "work,caches", "1"));
        let h2 = scope.spawn(|| run_dir(&temp, "fast", "Append", &fast_mod, &handler, "work,caches", "1"));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    let (declined, fast) = (open_out(&declined_out), open_out(&fast_out));
    let (found, _) = issues(&declined);
    let append = of(&found, "builtin declines");
    assert_eq!(append.len(), 1, "{found:#?}");
    assert!(!append[0].no_saving && append[0].ms.is_some(), "{:#?}", append[0]);
    assert!((append[0].upper_ms.unwrap() - code_per_frame(&declined)).abs() < SAME_MS);
    assert!(append[0].cost_text().contains("recorded work"));
    let saved = code_per_frame(&declined) - code_per_frame(&fast);
    assert!(saved > 0.0 && saved <= append[0].upper_ms.unwrap() + SAME_MS, "A/B saving {saved} is bounded");
    assert_accounted(&declined, &found);
    println!("real append fast path: A/B saves {saved:.12} ms/frame");
}

#[test]
fn real_comparator_declines_intersect_only_the_sort_context() {
    let temp = Temp::new("planted-context-union");
    // Compare is the sort's comparator, and also runs on its own in Outside: only its calls under the sort are the sort's
    let source = r#"local M = {}
local values = {}
for i = 1, 1000 do values[i] = tostring((i * 7919) % 10007 - 5000) end
function M.Compare(a, b)
    return math.abs(a) < math.abs(b)
end
function M.Sort()
    for i = 1, 500 do values[i], values[1001-i] = values[1001-i], values[i] end
    table.sort(values, M.Compare)
    return #values
end
function M.Outside()
    local sum = 0
    for i = 1, 10000 do if M.Compare("123", "54") then sum += 1 end end
    return sum
end
return M
"#;
    let handler = harness("Nested", "sink += M.Sort() + M.Outside()");
    let (_, app) = run(&temp, "contexts", "Nested", source, &handler, "work,timeline,caches", "2");
    let (found, _) = issues(&app);
    let sort = of(&found, "sort calls")[0];
    let decline = of(&found, "builtin declines")[0];
    let profile = app.profile();
    let frames = profile.scope_frames(&profile.scope()).unwrap();
    let has_fractional_microseconds = profile.raw.tree.iter().any(|node| (node.3 - node.3.round()).abs() > 1e-6);
    assert!(has_fractional_microseconds, "exact unions need fractional microseconds");

    // The decline's saving inside the sort's subtree is shared with the sort; its saving outside is its own
    let saving_per_frame = |inside_sort: bool| -> f64 {
        let sites = decline.work.sites.iter().filter(|site| sort.work.nodes.contains(&site.node) == inside_sort);
        sites.map(|site| site.saving).sum::<f64>() / frames
    };
    let inside = saving_per_frame(true);
    let outside = saving_per_frame(false);
    assert!(inside > 0.0 && outside > 0.0, "{decline:#?}");
    assert!(sort.overlap > 0.0 && decline.overlap > 0.0);
    let node_work: Vec<f64> = profile.raw.tree.iter().map(|node| node.3 / 1000.0).collect();
    let mut union = sort.work.clone();
    union.union(&decline.work);
    let expected = sort.ms.unwrap() + decline.ms.unwrap() - inside;
    assert!((union.total(&node_work) / frames - expected).abs() < 1e-8);
    assert_accounted(&app, &found);
    println!("real partial comparator: sort {:.12}, decline {:.12}, inside {inside:.12}, outside {outside:.12}, union {expected:.12} ms/frame", sort.ms.unwrap(), decline.ms.unwrap());
}
