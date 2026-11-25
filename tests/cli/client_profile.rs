//! The client profile through the built `ree`: the jobs of one `ree run --batch` (one process, its caches shared) each
//! answer by their own `--client-profile`, and a profile file named relative to the caller is read from there.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("ree-client-profile-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("empty")).unwrap();
    root
}

fn job(root: &Path, harness: &Path, id: &str, extra: &[&str]) -> serde_json::Value {
    let mut args: Vec<String> = ["--harness", &harness.to_string_lossy(), "--roots", &root.join("empty").to_string_lossy(), "--mode", "play", "--seconds", "1"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    args.extend(extra.iter().map(|s| s.to_string()));
    serde_json::json!({ "id": id, "args": args })
}

#[test]
fn batch_jobs_keep_their_own_client_profiles() {
    let root = scratch("batch");
    let harness = root.join("Who.luau");
    std::fs::write(&harness, "return function() local P = game:GetService('Players').LocalPlayer print(P.UserId, game:GetService('UserInputService').TouchEnabled, game.PlaceId) end").unwrap();
    std::fs::write(root.join("mine.json"), r#"{ "identity": { "userId": 23 }, "game": { "placeId": 9 } }"#).unwrap();
    let spec = serde_json::json!({ "workers": 2, "jobs": [
        job(&root, &harness, "phone", &["--client-profile", r#"{"identity":{"userId":21},"device":{"kind":"phone"}}"#]),
        job(&root, &harness, "desktop", &["--client-profile", r#"{"identity":{"userId":22}}"#]),
        job(&root, &harness, "file", &["--client-profile", "mine.json"]),
        job(&root, &harness, "none", &[]),
    ] });
    let file = root.join("jobs.json");
    std::fs::write(&file, spec.to_string()).unwrap();
    // (the job's relative profile file is read from the caller's folder; the state stays in this test's folder)
    let output = Command::new(env!("CARGO_BIN_EXE_ree"))
        .args(["run", "--batch"])
        .arg(&file)
        .current_dir(&root)
        .env("RUR_STATE_DIR", root.join("state"))
        .env_remove("REE_CLIENT_PROFILE")
        .env_remove("RUR_PLAYER_ID")
        .env_remove("RUR_PLACE_ID")
        .env_remove("REE_DEVICE")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let line = |index: usize| report["results"][index]["summary"]["output"][0]["message"].as_str().unwrap_or_default().to_string();
    assert_eq!([line(0), line(1), line(2), line(3)], ["21 true 0", "22 false 0", "23 false 9", "1 false 0"], "{report}");
    assert_eq!(report["results"][0]["summary"]["clientProfile"]["sources"], serde_json::json!(["inline"]));
    // (a file layer is named by its absolute path, as a service is given it)
    let named = report["results"][2]["summary"]["clientProfile"]["sources"][0].as_str().unwrap_or_default().to_string();
    assert!(Path::new(&named).is_absolute() && named.ends_with("mine.json"), "{named}");
    assert!(report["results"][3]["summary"].get("clientProfile").is_none());
    let _ = std::fs::remove_dir_all(&root);
}
