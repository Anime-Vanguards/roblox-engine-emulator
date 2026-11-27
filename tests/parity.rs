//! Differential tests against the real engine (tools/parity, README.md there): every probe with a captured expected
//! output runs through the built `ree`, and every expected line it does not print must be listed in known.json as a
//! known difference. A fix that makes a known line match passes too; `node tools/parity/run.mjs` says which entries
//! to remove. The server's set (expected/server, known-server.json) runs the same way with `--run-context server`, and the
//! paired set (probes/loopback: a server and its client in one play session; expected/loopback, known-loopback.json) with
//! `--run-context loopback`, each probe's two sides as its harness's Server and Client. The animation ids probes load play
//! the stand-in clips in tools/parity/animations (`--animations`), so no run needs asset access; where this machine fetches
//! the real assets, the probes that load them are also checked on those.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

fn scratch_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let seq = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ree-parity-{label}-{}-{nanos}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs a batch of `ree run` jobs in one process so probes share compiled caches and run across worker threads.
fn run_batch(scratch: &Path, jobs: &[serde_json::Value]) -> Vec<serde_json::Value> {
    if jobs.is_empty() {
        return Vec::new();
    }
    let batch_file = scratch.join("batch.json");
    let spec = serde_json::json!({ "jobs": jobs });
    std::fs::write(&batch_file, serde_json::to_string(&spec).unwrap()).unwrap();
    let done = Command::new(env!("CARGO_BIN_EXE_ree"))
        .args(["run", "--batch"])
        .arg(&batch_file)
        .env("RUR_STATE_DIR", scratch.join("state"))
        .env_remove("REE_FROZEN_CLOCK")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&done.stdout);
    let report: serde_json::Value = serde_json::from_str(stdout.trim().lines().last().unwrap_or("{}"))
        .unwrap_or_else(|_| panic!("batch produced no JSON report (exit {:?}): {}", done.status.code(), String::from_utf8_lossy(&done.stderr)));
    report["results"].as_array().unwrap_or_else(|| panic!("batch missing results: {report}")).clone()
}

fn printed_lines(summary: &serde_json::Value) -> Vec<String> {
    let mut lines = Vec::new();
    for entry in summary["output"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let message = entry["message"].as_str().unwrap_or_default();
        if entry["kind"] == "print" && message != "@done" {
            lines.extend(message.split('\n').map(str::to_owned));
        }
    }
    lines
}

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools").join("parity")
}

/// The clips the probes' animation ids load in a parity run (`--animations`, tools/parity/README.md "Animation assets"):
/// made for the tests with what the probes read of the real assets, so a run with no asset access (a CI runner) prints
/// what the real engine printed.
fn animations_dir() -> PathBuf {
    parity_dir().join("animations")
}

/// The asset ids tools/parity/animations stands in for (its `<id>.rbxmx` files)
fn stand_in_ids() -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(animations_dir())
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.strip_suffix(".rbxmx").map(str::to_owned))
        .collect();
    ids.sort();
    ids
}

/// The text both engines run (tools/parity/run.mjs `body`).
fn body(prelude: &str, probe: &str) -> String {
    format!(
        "{prelude}\nP.base = debug.info(1, \"l\") local __ParityOk, __ParityError = pcall(function()\n{probe}\nend)\nif not __ParityOk then\n\tprint(\"probe-error \" .. P.err(__ParityError))\nend\n"
    )
}

fn expected_lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n").lines().filter(|line| !line.starts_with("# ")).map(str::to_owned).collect()
}

/// Expected lines missing from `actual`, in order (the lines a longest common subsequence leaves out).
fn missing(expected: &[String], actual: &[String]) -> Vec<String> {
    let (n, m) = (expected.len(), actual.len());
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if expected[i] == actual[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n {
        if j < m && expected[i] == actual[j] {
            i += 1;
            j += 1;
        } else if j < m && table[i][j + 1] >= table[i + 1][j] {
            j += 1;
        } else {
            out.push(expected[i].clone());
            i += 1;
        }
    }
    out
}

/// The probe's print collects its lines and the harness prints them when the probe has ended, as the real engine's test
/// runner does, so they never reach LogService.MessageOut (tools/parity/run.mjs HARNESS_PRINT and HARNESS_FLUSH: the same
/// text).
const HARNESS_PRINT: &str = "local __ParityLines, __ParityRealPrint = {}, print\nlocal function print(...)\n\tlocal Parts = table.pack(...)\n\tfor Index = 1, Parts.n do\n\t\tParts[Index] = tostring(Parts[Index])\n\tend\n\ttable.insert(__ParityLines, table.concat(Parts, \" \", 1, Parts.n))\nend";
const HARNESS_FLUSH: &str = "for _, Line in __ParityLines do\n\t__ParityRealPrint(Line)\nend\n__ParityRealPrint(\"@done\")";

fn probe_job(scratch: &Path, empty: &Path, name: &str, source: &str, server: bool, stand_ins: bool) -> serde_json::Value {
    let harness = scratch.join(format!("{name}.luau"));
    std::fs::write(&harness, format!("return function(Context)\n{HARNESS_PRINT}\n{source}\n{HARNESS_FLUSH}\nend\n")).unwrap();
    // (the server capture's place has ServerScriptService.LoadStringEnabled on; a probe's "flag=" names an engine flag it needs on;
    // "context=edit" runs it as a plugin's code, in the edit run context: run.mjs runInRee)
    let directive = |word: &str| source.lines().any(|line| line.starts_with("-- parity:") && line.split_whitespace().any(|part| part == word));
    let mut args: Vec<String> = vec![
        "--harness".into(),
        harness.to_string_lossy().into_owned(),
        "--roots".into(),
        empty.to_string_lossy().into_owned(),
        // (300 s of the run's clock, the capture's cap on one job: run.mjs runInRee)
        "--seconds".into(),
        "300".into(),
        "--max-wall-seconds".into(),
        "60".into(),
    ];
    if stand_ins {
        args.extend(["--animations".into(), animations_dir().to_string_lossy().into_owned()]);
    }
    if server {
        args.extend(["--run-context", "server", "--load-string"].map(str::to_owned));
    } else if directive("context=edit") {
        args.extend(["--mode", "edit", "--run-context", "edit"].map(str::to_owned));
    }
    if directive("flag=FFlagEnableLoadModule") {
        args.push("--load-module".into());
    }
    // ("studio=false": what a running game refuses that Studio allows: run.mjs runInRee)
    if directive("studio=false") {
        args.extend(["--studio", "false"].map(str::to_owned));
    }
    // ("timeout=<seconds>": the script timeout the capture set with ScriptContext:SetTimeout: run.mjs runInRee)
    let timeout = source.lines().filter(|line| line.starts_with("-- parity:")).flat_map(str::split_whitespace).find_map(|part| part.strip_prefix("timeout="));
    if let Some(seconds) = timeout {
        args.extend(["--script-timeout", seconds, "--max-script-timeouts", "0"].map(str::to_owned));
    }
    serde_json::json!({ "id": name, "args": args })
}

/// Each side's print collects its lines (tools/parity/run.mjs LOOPBACK_COLLECT: the same text)
const LOOPBACK_COLLECT: &str = "local __LoopbackLines, __LoopbackRealPrint = {}, print\nlocal function print(...)\n\tlocal Parts = table.pack(...)\n\tfor Index = 1, Parts.n do\n\t\tParts[Index] = tostring(Parts[Index])\n\tend\n\ttable.insert(__LoopbackLines, table.concat(Parts, \" \", 1, Parts.n))\nend";

/// One side of a paired probe (tools/parity/run.mjs `loopbackText`): the file with the other side's lines blanked, wrapped so
/// the client prints both sides' lines at the end (loopback.luau)
fn loopback_text(prelude: &str, helpers: &str, probe: &str, side: &str) -> String {
    let mut section = "both";
    let kept: Vec<&str> = probe
        .split('\n')
        .map(|line| {
            let line = line.trim_end_matches('\r');
            match line.strip_prefix("--- ").map(str::trim_end) {
                Some(marker @ ("server" | "client")) => {
                    section = marker;
                    line
                }
                _ if section == "both" || section == side => line,
                _ => "",
            }
        })
        .collect();
    format!(
        "{LOOPBACK_COLLECT}\n{prelude}\n{helpers}\nP.base = debug.info(1, \"l\") local __ParityOk, __ParityError = pcall(function()\n{}\nend)\nif not __ParityOk then\n\tprint(\"probe-error \" .. P.err(__ParityError))\nend\n__LoopbackFinish(__LoopbackLines, __LoopbackRealPrint)\n",
        kept.join("\n")
    )
}

/// A paired probe's batch job for `ree run --run-context loopback`
fn loopback_probe_job(scratch: &Path, empty: &Path, name: &str, server: &str, client: &str) -> serde_json::Value {
    let harness = scratch.join(format!("{name}.luau"));
    std::fs::write(&harness, format!("return {{\n\tServer = function(Context)\n{server}\nend,\n\tClient = function(Context)\n{client}\n__LoopbackRealPrint(\"@done\")\nend,\n}}\n")).unwrap();
    serde_json::json!({
        "id": name,
        "args": [
            "--harness", harness.to_string_lossy().into_owned(),
            "--roots", empty.to_string_lossy().into_owned(),
            "--seconds", "120",
            "--max-wall-seconds", "60",
            "--run-context", "loopback",
            "--animations", animations_dir().to_string_lossy().into_owned(),
        ]
    })
}

/// The paired probes (probes/loopback): a server and its client in one play session, as a real play session's two sides ran
/// them (capture_sim.py), ree with --run-context loopback. Every probe has its captured output.
#[test]
fn paired_probes_print_what_a_real_server_and_client_printed() {
    let dir = parity_dir();
    let prelude = std::fs::read_to_string(dir.join("prelude.luau")).unwrap();
    let helpers = std::fs::read_to_string(dir.join("loopback.luau")).unwrap();
    let known: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("known-loopback.json")).unwrap()).unwrap();
    let scratch = scratch_dir("loopback");
    let empty = scratch.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let mut names: Vec<String> = std::fs::read_dir(dir.join("probes").join("loopback"))
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.strip_suffix(".luau").map(str::to_owned))
        .collect();
    names.sort();
    let mut failures = Vec::new();
    let mut queued: Vec<(&str, Vec<String>)> = Vec::new();
    let mut jobs = Vec::new();
    for name in &names {
        let Ok(expected) = std::fs::read_to_string(dir.join("expected").join("loopback").join(format!("{name}.txt"))) else {
            failures.push(format!("{name}: no captured output (expected/loopback/{name}.txt: run.mjs --loopback --capture {name})"));
            continue;
        };
        let expected = expected_lines(&expected);
        let probe = std::fs::read_to_string(dir.join("probes").join("loopback").join(format!("{name}.luau"))).unwrap();
        let server = loopback_text(&prelude, &helpers, &probe, "server");
        let client = loopback_text(&prelude, &helpers, &probe, "client");
        jobs.push(loopback_probe_job(&scratch, &empty, name, &server, &client));
        queued.push((name.as_str(), expected));
    }
    let results = run_batch(&scratch, &jobs);
    assert_eq!(results.len(), queued.len());
    for ((name, expected), result) in queued.into_iter().zip(results.iter()) {
        assert_eq!(result["id"].as_str(), Some(name));
        assert_ne!(result["code"].as_i64(), Some(124), "{name}: probe hit its wall-clock cap: {result}");
        let actual = printed_lines(&result["summary"]);
        let allowed: HashSet<&str> =
            known[name].as_array().map(|list| list.iter().filter_map(|entry| entry["line"].as_str()).collect()).unwrap_or_default();
        for line in missing(&expected, &actual) {
            if !allowed.iter().any(|prefix| line.starts_with(prefix)) {
                failures.push(format!("{name}: the real engine printed `{line}`"));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(!names.is_empty(), "no paired probe");
    assert!(failures.is_empty(), "{} new differences from the real engine (node tools/parity/run.mjs --loopback --diff):\n{}", failures.len(), failures.join("\n"));
}

/// The comparison catches what a broken bridge would print: two messages in the other order, a server-only child that
/// reached the client, an echo that never came back (each a captured line changed as it would be)
#[test]
fn the_paired_comparison_catches_a_swap_a_leak_and_a_lost_echo() {
    let dir = parity_dir().join("expected").join("loopback");
    let read = |name: &str| expected_lines(&std::fs::read_to_string(dir.join(format!("{name}.txt"))).unwrap());
    let changed = |name: &str, find: &dyn Fn(&str) -> bool, change: &dyn Fn(&str) -> String| {
        let expected = read(name);
        assert!(missing(&expected, &expected).is_empty());
        let at = expected.iter().position(|line| find(line)).unwrap_or_else(|| panic!("{name}: no line to change"));
        let mut actual = expected.clone();
        actual[at] = change(&actual[at]);
        assert_ne!(actual[at], expected[at]);
        missing(&expected, &actual)
    };
    // the order the server's remotes arrived in, two of them swapped
    let swapped = changed("replication_order", &|line| line.starts_with("client: client got "), &|line| line.replacen("A1 B2", "B2 A1", 1));
    assert_eq!(swapped.len(), 1, "{swapped:?}");
    // a marker the server put into ServerStorage, listed among the client's
    let leaked = changed("replicated_services", &|line| line.starts_with("client: MarkerBeforeJoin arrived in "), &|line| format!("{line} ServerStorage"));
    assert_eq!(leaked.len(), 1, "{leaked:?}");
    // one echo of sixty lost
    let lost = changed("remote_latency", &|line| line.contains("event round trip: 60 back"), &|line| line.replacen("60 back", "59 back", 1));
    assert_eq!(lost.len(), 1, "{lost:?}");
    // two lines in the other order: one of them is reported
    let expected = read("remote_values");
    let mut actual = expected.clone();
    let first = actual.iter().position(|line| line.starts_with("client: case 1 ")).unwrap();
    actual.swap(first, first + 1);
    assert_eq!(missing(&expected, &actual).len(), 1);
}

#[test]
fn probes_print_what_the_real_engine_printed() {
    check(false, false);
}

/// The probes run as a server's script (a Studio 0.744 play session's server: capture_sim.py), ree with --run-context server
#[test]
fn probes_print_on_a_server_what_a_real_server_printed() {
    check(true, false);
}

/// The probes that load an asset tools/parity/animations stands in for, run on the real asset instead (no `--animations`),
/// on a client and on a server, where this machine fetches it (the download worker wrote it into the run's cache): what
/// keeps each stand-in true to its asset. Where it is not fetched (no worker or no network, as on a CI runner) such a probe
/// is not compared, and the test says so on stderr.
#[test]
fn probes_print_with_the_real_animation_assets_what_they_print_with_the_stand_ins() {
    check(false, true);
    check(true, true);
}

/// Runs the probes with a captured output and fails on any expected line ree does not print that the known list does not
/// allow: with the stand-in animation clips, or (`real_assets`) only the probes that name a stand-in's id, without them.
fn check(server: bool, real_assets: bool) {
    let dir = parity_dir();
    let prelude = std::fs::read_to_string(dir.join("prelude.luau")).unwrap();
    let known_file = if server { "known-server.json" } else { "known.json" };
    let known: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(known_file)).unwrap()).unwrap();
    let expected_dir = if server { dir.join("expected").join("server") } else { dir.join("expected") };
    let side = if server { "server" } else { "client" };
    let scratch = scratch_dir(side);
    let empty = scratch.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let mut names: Vec<String> = std::fs::read_dir(dir.join("probes"))
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.strip_suffix(".luau").map(str::to_owned))
        .collect();
    names.sort();
    let stand_ins = stand_in_ids();
    let mut queued: Vec<(&str, Vec<String>, Vec<&str>)> = Vec::new();
    let mut jobs = Vec::new();
    for name in &names {
        let Ok(expected) = std::fs::read_to_string(expected_dir.join(format!("{name}.txt"))) else { continue };
        let expected = expected_lines(&expected);
        let probe = std::fs::read_to_string(dir.join("probes").join(format!("{name}.luau"))).unwrap();
        let assets: Vec<&str> = stand_ins.iter().map(String::as_str).filter(|id| probe.contains(id)).collect();
        if real_assets && assets.is_empty() {
            continue;
        }
        jobs.push(probe_job(&scratch, &empty, name, &body(&prelude, &probe), server, !real_assets));
        queued.push((name.as_str(), expected, assets));
    }
    let results = run_batch(&scratch, &jobs);
    assert_eq!(results.len(), queued.len());
    let mut failures = Vec::new();
    let mut unfetched = Vec::new();
    let checked = queued.len();
    // (the download cache of the batch's state dir: animation/source.rs)
    let fetched = |id: &str| scratch.join("state").join("animations").join(format!("{id}.rbxm")).is_file();
    for ((name, expected, assets), result) in queued.into_iter().zip(results.iter()) {
        assert_eq!(result["id"].as_str(), Some(name));
        assert_ne!(result["code"].as_i64(), Some(124), "{name}: probe hit its wall-clock cap: {result}");
        let summary = &result["summary"];
        assert_eq!(summary["done"], true, "{name}: the probe did not finish: {result}");
        if real_assets && !assets.iter().all(|id| fetched(id)) {
            unfetched.push(format!("{name} ({})", assets.join(", ")));
            continue;
        }
        let actual = printed_lines(summary);
        let allowed: HashSet<&str> =
            known[name].as_array().map(|list| list.iter().filter_map(|entry| entry["line"].as_str()).collect()).unwrap_or_default();
        for line in missing(&expected, &actual) {
            if !allowed.iter().any(|prefix| line.starts_with(prefix)) {
                failures.push(format!("{name}: the real engine printed `{line}`"));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    if !unfetched.is_empty() {
        eprintln!("{side}: not compared on the real animation assets, which this machine did not fetch (they ran on the stand-ins in tools/parity/animations): {}", unfetched.join(", "));
    }
    assert!(checked > 0, "{}", if real_assets { "no probe with an expected output loads an asset tools/parity/animations stands in for" } else { "no probe has an expected output" });
    let command = match (server, real_assets) {
        (false, false) => "node tools/parity/run.mjs --diff",
        (true, false) => "node tools/parity/run.mjs --server --diff",
        (false, true) => "on the real animation assets: node tools/parity/run.mjs --real-assets --diff",
        (true, true) => "on the real animation assets: node tools/parity/run.mjs --real-assets --server --diff",
    };
    assert!(failures.is_empty(), "{} new differences from the real engine ({command}):\n{}", failures.len(), failures.join("\n"));
}
