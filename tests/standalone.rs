//! The same offline smoke as a release runner, with only ree copied outside its build checkout.

use std::path::Path;
use std::process::{Command, Output};

/// Runs `node <script> <args>` with the temporary folder moved to `temp`, so a test can see what the script leaves there.
/// The scripts that copy and run ree take the binary under test from `REE_TEST_BINARY`.
fn node(script: &str, args: &[&str], temp: &Path) -> Output {
    Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(script))
        .args(args)
        .env("TMPDIR", temp).env("TMP", temp).env("TEMP", temp)
        .env("REE_TEST_BINARY", env!("CARGO_BIN_EXE_ree"))
        .output()
        .expect("Node.js runs the script")
}

fn describe(output: &Output) -> String {
    format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// Prints the lines where a script gave up removing its temporary folder, and says whether there were any.
///
/// On Windows a process can hold a folder briefly after it exits. The script retries, then keeps the folder and says so.
/// That must not fail a test whose checks passed, but the test must then leave the folder alone and not count its files.
fn report_deferred_cleanup(output: &Output) -> bool {
    let diagnostic = describe(output);
    let mut deferred = false;
    for line in diagnostic.lines().filter(|line| line.contains("cleanup could not remove ")) {
        eprintln!("{line}");
        deferred = true;
    }
    deferred
}

// The only test here that reads or sets the default repo: both are once per process.
#[test]
fn a_host_can_replace_the_detected_checkout_with_its_own_repo() {
    use roblox_engine_emulator::{Options, default_repo, set_default_repo};
    let host = std::env::temp_dir().join("ree-host-checkout");
    set_default_repo(host.clone());
    assert_eq!(default_repo(), host);
    assert_eq!(Options::default().repo, host, "the options default to the host's repo");
    let other = std::env::temp_dir().join("ree-other-host-checkout");
    assert_eq!(Options { repo: other.clone(), ..Options::default() }.repo, other, "a host can still pass a repo per run");
}

#[test]
fn an_installed_binary_and_its_service_use_only_the_redirected_user_cache() {
    let temp = std::env::temp_dir().join(format!("ree-smoke-test-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    // the smoke compares the paths it made with the ones the binary reports, so the root must not be a link
    let temp = std::fs::canonicalize(temp).unwrap();
    let binary = env!("CARGO_BIN_EXE_ree");

    // a passing smoke removes its folder
    let passed = node("tools/release/smoke.mjs", &["--binary", binary, "--version", env!("CARGO_PKG_VERSION")], &temp);
    assert!(passed.status.success(), "{}", describe(&passed));
    let cleanup_deferred = report_deferred_cleanup(&passed);
    if !cleanup_deferred {
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0, "a passing smoke must clean {}", temp.display());
    }

    // a failing smoke (the wrong version) keeps its folder and says where it is
    let failed = node("tools/release/smoke.mjs", &["--binary", binary, "--version", "wrong-version"], &temp);
    assert!(!failed.status.success());
    // the folder of the passing run may still be there too (when its removal was deferred), so any line may be the one named;
    // Node prints an ordinary Windows path where `temp` is canonical with a verbatim prefix, so the paths are compared canonical
    let diagnostic = String::from_utf8_lossy(&failed.stderr);
    let kept_folders = diagnostic.lines().filter_map(|line| line.strip_prefix("release smoke failed; files kept at "));
    let names_a_kept_folder = kept_folders.filter_map(|kept| std::fs::canonicalize(kept).ok())
        .any(|kept| kept.is_dir() && kept.parent() == Some(temp.as_path()));
    assert!(names_a_kept_folder);
    if !cleanup_deferred {
        std::fs::remove_dir_all(&temp).unwrap();
    }
}

#[cfg(feature = "bundles")]
#[test]
fn asset_workers_keep_the_clients_paths_through_the_service() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/asset_worker.mjs");
    let result = Command::new("node").arg("--test").arg(script)
        .env("REE_TEST_BINARY", env!("CARGO_BIN_EXE_ree"))
        .output().expect("Node.js runs the asset worker tests");
    assert!(result.status.success(), "{}", describe(&result));
    report_deferred_cleanup(&result);
}

#[test]
fn installed_jobs_keep_standalone_state_with_an_input_repo() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/service_cache.mjs");
    let result = Command::new("node").arg("--test").arg(script)
        .env("REE_TEST_BINARY", env!("CARGO_BIN_EXE_ree"))
        .output().expect("Node.js runs the service cache tests");
    assert!(result.status.success(), "{}", describe(&result));
    report_deferred_cleanup(&result);
}

#[test]
fn standalone_checks_accept_linked_and_canonical_temporary_folders() {
    // Some systems reach their temporary folder through a link (macOS /var, a Windows junction). The checks build their
    // own paths in it and compare them with what the binary reports, so they must work when the folder is a link.
    let root = std::env::temp_dir().join(format!("ree-linked-temp-{}", std::process::id()));
    let real = root.join("real");
    let linked = root.join("linked temp");
    std::fs::create_dir_all(&real).unwrap();
    // A junction needs no elevated privileges on Windows; Unix uses a directory symlink.
    let link_script = "require('node:fs').symlinkSync(process.argv[1], process.argv[2], process.platform === 'win32' ? 'junction' : 'dir')";
    let setup = Command::new("node").arg("-e").arg(link_script).arg(&real).arg(&linked).output().unwrap();
    assert!(setup.status.success(), "{}", describe(&setup));
    // Rust's canonical Windows path has a verbatim prefix; Node must accept that spelling too.
    let canonical = std::fs::canonicalize(&linked).unwrap();

    let smoke_args = ["--binary", env!("CARGO_BIN_EXE_ree"), "--version", env!("CARGO_PKG_VERSION")];
    let mut checks: Vec<(&str, &[&str])> = vec![("tests/service_cache.mjs", &[]), ("tools/release/smoke.mjs", &smoke_args)];
    // the asset worker needs the bundles feature
    if cfg!(feature = "bundles") {
        checks.push(("tests/asset_worker.mjs", &[]));
    }
    let runs: Vec<(&str, &[&str], &Path)> = checks
        .into_iter()
        .flat_map(|(script, args)| [linked.as_path(), canonical.as_path()].into_iter().map(move |temp| (script, args, temp)))
        .collect();
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    let concurrency = if cpus >= 8 { (cpus / 4).min(6) } else { 1 };
    let results: Vec<(&str, &Path, Output)> = if concurrency <= 1 {
        runs.into_iter().map(|(script, args, temp)| (script, temp, node(script, args, temp))).collect()
    } else {
        let queue = std::sync::Mutex::new(runs.into_iter().enumerate());
        let slots: std::sync::Mutex<Vec<Option<(&str, &Path, Output)>>> = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..concurrency)
                .map(|_| {
                    let queue = &queue;
                    let slots = &slots;
                    scope.spawn(move || loop {
                        let next = queue.lock().unwrap().next();
                        let Some((index, (script, args, temp))) = next else { break };
                        let output = node(script, args, temp);
                        let mut guard = slots.lock().unwrap();
                        if guard.len() <= index {
                            guard.resize_with(index + 1, || None);
                        }
                        guard[index] = Some((script, temp, output));
                    })
                })
                .collect();
            for handle in handles {
                handle.join().unwrap();
            }
        });
        slots.into_inner().unwrap().into_iter().map(Option::unwrap).collect()
    };
    let mut cleanup_deferred = false;
    for (script, temp, result) in results {
        assert!(result.status.success(), "{script} with {}: {}", temp.display(), describe(&result));
        cleanup_deferred |= report_deferred_cleanup(&result);
    }
    if !cleanup_deferred {
        assert_eq!(std::fs::read_dir(&real).unwrap().count(), 0, "passing checks clean up through the link");
    }
    // the link first, so that removing the root does not follow it: a Unix symlink is removed as a file, a Windows junction as a directory
    #[cfg(unix)]
    std::fs::remove_file(&linked).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(&linked).unwrap();
    if !cleanup_deferred {
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[test]
fn run_batch_executes_jobs_in_parallel_and_reports_dependencies() {
    let root = std::env::temp_dir().join(format!("ree-run-batch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let source_dir = root.join("src").join("ReplicatedStorage");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(source_dir.join("Helper.luau"), "return { Double = function(x) return x * 2 end }\n").unwrap();
    std::fs::write(source_dir.join("Config.json"), r#"{"Offset": 7}"#).unwrap();
    let harness = root.join("harness.luau");
    std::fs::write(
        &harness,
        r#"return function(Context)
    local RS = game:GetService("ReplicatedStorage")
    local Helper = require(RS.Helper)
    local Config = debug.loadmodule(RS.Config)()
    print("@res " .. tostring(Helper.Double(Context.Input) + Config.Offset))
    print("@done")
end
"#,
    )
    .unwrap();
    let ctx1 = root.join("ctx1.json");
    let ctx2 = root.join("ctx2.json");
    std::fs::write(&ctx1, r#"{"Input": 5}"#).unwrap();
    std::fs::write(&ctx2, r#"{"Input": 20}"#).unwrap();
    let batch_file = root.join("batch.json");
    let common = |ctx: &Path| {
        vec![
            "--native".to_string(),
            "--harness".to_string(),
            harness.to_string_lossy().into_owned(),
            "--roots".to_string(),
            root.join("src").to_string_lossy().into_owned(),
            "--mode".to_string(),
            "edit".to_string(),
            "--run-context".to_string(),
            "edit".to_string(),
            "--load-module".to_string(),
            "--offline".to_string(),
            "--seconds".to_string(),
            "10".to_string(),
            "--context".to_string(),
            ctx.to_string_lossy().into_owned(),
        ]
    };
    let spec = serde_json::json!({
        "workers": 2,
        "jobs": [
            { "id": "first", "args": common(&ctx1), "dependencies": true },
            { "id": "second", "args": common(&ctx2), "dependencies": false },
        ]
    });
    std::fs::write(&batch_file, serde_json::to_string(&spec).unwrap()).unwrap();

    // The binary under test is the build's own: in its checkout its state is the checkout's `.cache` (built elsewhere,
    // the user's ree cache), and the release smoke running beside this test fails when the checkout changes. The jobs'
    // module lists (record/<hash>.txt) go to this test's folder instead.
    let state = root.join("state");
    let output = Command::new(env!("CARGO_BIN_EXE_ree"))
        .args(["run", "--batch"])
        .arg(&batch_file)
        .env("RUR_STATE_DIR", &state)
        .output()
        .expect("ree run --batch executes");
    assert!(output.status.success(), "{}", describe(&output));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid batch JSON");
    let results = report["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["id"], "first");
    assert_eq!(results[1]["id"], "second");
    let first_msg = results[0]["summary"]["output"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|entry| entry["message"].as_str().filter(|m| m.starts_with("@res ")));
    let second_msg = results[1]["summary"]["output"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|entry| entry["message"].as_str().filter(|m| m.starts_with("@res ")));
    assert_eq!(first_msg, Some("@res 17"));
    assert_eq!(second_msg, Some("@res 47"));
    let deps = results[0]["dependencies"].as_array().expect("dependencies reported for first job");
    assert!(deps.iter().any(|p| p.as_str().is_some_and(|s| s.ends_with("Helper.luau"))), "{deps:?}");
    assert!(deps.iter().any(|p| p.as_str().is_some_and(|s| s.ends_with("Config.json"))), "{deps:?}");
    assert!(results[1].get("dependencies").is_none());
    let lists = std::fs::read_dir(state.join("record")).map(|entries| entries.count()).unwrap_or(0);
    assert!(lists > 0, "the batch's module lists belong in the redirected state dir {}", state.display());
    let _ = std::fs::remove_dir_all(&root);
}
