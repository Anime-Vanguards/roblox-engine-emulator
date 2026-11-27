//! The client profile (client_profile.rs) through the script API: what every reader answers without a profile (the engine as
//! it was), what each field changes, a server's starting data and its answers for the profile's player, a loopback pair, the
//! harness's Context.ClientProfile / SetClientProfile and the run's summary. Each test runs a harness through `record::run`
//! as `ree run` does.

use std::path::PathBuf;

use crate::{
    launch::Host,
    record::run::{self, RecordArgs},
};

/// A scratch folder of its own for a test
#[inline(never)]
fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ree-client-profile-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The arguments of `ree run --harness <file> --mount ReplicatedStorage/Empty=<dir> --seconds 2 --mode play` and `extra`
fn words(file: &std::path::Path, empty: &std::path::Path, extra: &[&str]) -> Vec<String> {
    let mut words: Vec<String> = ["--harness", &file.to_string_lossy(), "--mount", &format!("ReplicatedStorage/Empty={}", empty.display()), "--seconds", "2", "--mode", "play"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    words.extend(extra.iter().map(|s| s.to_string()));
    words
}

/// `ree run` of `harness` with `extra` arguments: its summary, or the error that kept it from running
#[inline(never)]
fn try_run(name: &str, harness: &str, extra: &[&str]) -> Result<serde_json::Value, String> {
    let dir = scratch(name);
    let file = dir.join(format!("{name}.luau"));
    std::fs::write(&file, harness).unwrap();
    std::fs::create_dir_all(dir.join("empty")).unwrap();
    let result = run::parse(&words(&file, &dir.join("empty"), extra), false).and_then(|args: RecordArgs| run::run_only(&args, &Host::default()));
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[inline(never)]
fn run_harness(name: &str, harness: &str, extra: &[&str]) -> serde_json::Value {
    try_run(name, harness, extra).unwrap_or_else(|error| panic!("{error}"))
}

/// Every output line of a summary as `kind message`
#[inline(never)]
fn every_line(summary: &serde_json::Value) -> Vec<String> {
    summary["output"].as_array().unwrap().iter().map(|line| format!("{} {}", line["kind"].as_str().unwrap_or("?"), line["message"].as_str().unwrap_or(""))).collect()
}

/// The printed lines of a summary (a loopback run's as `side: message`)
#[inline(never)]
fn printed(summary: &serde_json::Value) -> Vec<String> {
    summary["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|line| line["kind"] == "print")
        .map(|line| match line["side"].as_str() {
            Some(side) => format!("{side}: {}", line["message"].as_str().unwrap_or("")),
            None => line["message"].as_str().unwrap_or("").to_string(),
        })
        .collect()
}

/// `lines` match `expected` line by line: a line whose rest after its first word and label is JSON matches as JSON
/// (JSONEncode writes a table's keys in its own order), any other line as text
#[track_caller]
fn assert_lines(lines: &[String], expected: &[&str]) {
    let json = |line: &str| -> Option<(String, serde_json::Value)> {
        let at = line.find(['{', '['])?;
        let (label, rest) = line.split_at(at);
        serde_json::from_str(rest).ok().map(|value| (label.to_string(), value))
    };
    assert_eq!(lines.len(), expected.len(), "{lines:#?}");
    for (line, wanted) in lines.iter().zip(expected) {
        match (json(line), json(wanted)) {
            (Some(a), Some(b)) => assert_eq!(a, b, "{line}"),
            _ => assert_eq!(line, wanted),
        }
    }
}

/// Every reader of the profile a client's script reaches, one line each (a JSON value on a line of its own)
const CLIENT_READS: &str = include_str!("client_profile_tests/reads.luau");

/// What the base commit (main 056dc98, before client profiles) printed for CLIENT_READS, but for GuiService.PreferredTransparency
/// (1, not 0) and the policy table (all twelve keys, no external links), which follow a 0.747 client since (measured 2026-11-26)
const DEFAULT_LINES: [&str; 29] = [
    r#"print player Player Player 1 0 Enum.MembershipType.None false  0"#,
    r#"print inputs true true false false false false false"#,
    r#"print input Enum.PreferredInput.KeyboardAndMouse Enum.UserInputType.MouseMovement"#,
    r#"print gui false 0, 0 0, 0, 0, 0 Enum.PreferredTextSize.Medium 1 false"#,
    r#"print screen 1920, 1080 0, 0, 1920, 1080"#,
    r#"print settings Enum.SavedQualitySetting.Automatic 1"#,
    r#"print vr false"#,
    r#"print locale en-us en-us US"#,
    r#"print game 0 0 0 Enum.CreatorType.Group  0  0 false"#,
    r#"print policy {"AllowedExternalLinkReferences":0,"AreAdsAllowed":true,"ArePaidRandomItemsRestricted":false,"IsContentSharingAllowed":true,"IsEligibleToPurchaseCommerceProduct":false,"IsEligibleToPurchaseSubscription":true,"IsEndlessContentAutoplayAllowed":true,"IsEndlessContentLoadAllowed":true,"IsPaidItemTradingAllowed":true,"IsPhotoToAvatarAllowed":false,"IsSubjectToChinaPolicies":false,"IsThirdPartyAccountLinkingAllowed":true}"#,
    r#"print owns false false false false false"#,
    r#"missing MarketplaceService.PlayerOwnsAssetAsync is not emulated"#,
    r#"print owns async nil"#,
    r#"missing MarketplaceService.GetUserSubscriptionStatusAsync is not emulated"#,
    r#"print subscription null"#,
    r#"print badges false false"#,
    r#"missing BadgeService.CheckUserBadgesAsync is not emulated"#,
    r#"print badge list null"#,
    r#"print groups false 0 Guest false 0 Guest"#,
    r#"print group list []"#,
    r#"print friends false false"#,
    r#"print online []"#,
    r#"print friend pages true"#,
    r#"print friend page []"#,
    r#"print names Player John Doe 1 1"#,
    r#"print infos [{"Id":1,"Username":"Player","HasVerifiedBadge":false,"DisplayName":"Player"},{"Id":2,"Username":"John Doe","HasVerifiedBadge":false,"DisplayName":"John Doe"}]"#,
    r#"print blocked null"#,
    r#"print player calls false 0.05"#,
    r#"print @done"#,
];

/// Without a profile every reader answers as the engine did before profiles (DEFAULT_LINES, measured on the base commit);
/// the `default` preset is that profile
#[test]
fn without_a_profile_every_reader_answers_as_before() {
    let expected = &DEFAULT_LINES;
    let summary = run_harness("default", CLIENT_READS, &[]);
    assert_eq!(summary["status"], "ok", "{summary}");
    assert!(summary.get("clientProfile").is_none(), "no profile layer: no clientProfile");
    assert_lines(&every_line(&summary), expected);
    let preset = run_harness("default-preset", CLIENT_READS, &["--client-profile", "default"]);
    assert_lines(&every_line(&preset), expected);
    assert_eq!(preset["clientProfile"]["sources"], serde_json::json!(["default"]));
}

/// A profile that sets every field a client reads
const FULL: &str = r#"{
	"identity": { "name": "Ada", "displayName": "Ada L", "userId": 5, "accountAge": 400, "membershipType": "Premium", "hasVerifiedBadge": true, "verified": true, "followUserId": 9 },
	"device": { "kind": "phone", "inputs": { "accelerometer": true, "gyroscope": true, "vr": true }, "tenFootInterface": true, "viewport": [844, 390], "topbarInset": 58, "networkPing": 0.12 },
	"settings": { "preferredTextSize": "Large", "preferredTransparency": 0.5, "reducedMotion": true, "savedQualityLevel": "QualityLevel3", "cameraYInverted": true },
	"localization": { "localeId": "de-de", "robloxLocaleId": "de-de", "systemLocaleId": "de-de", "countryRegion": "DE" },
	"policy": { "AreAdsAllowed": false, "AllowedExternalLinkReferences": [] },
	"ownership": { "gamePasses": [11], "assets": [21], "bundles": "all", "badges": [41], "subscriptions": ["EXP-1"] },
	"social": {
		"groups": [{ "id": 7, "rank": 200, "role": "Admin", "name": "Seven" }],
		"friends": [{ "userId": 2, "name": "Bo", "displayName": "Bobo", "online": true, "presence": { "PlaceId": 3 } }, { "userId": 3, "name": "Cy" }],
		"blocked": [4, 6]
	},
	"game": { "placeId": 10, "gameId": 11, "creatorId": 12, "creatorType": "User", "jobId": "job-1", "placeVersion": 3, "privateServerId": "ps", "privateServerOwnerId": 13, "streamingTargetRadius": 400 }
}"#;

/// Each field changes what scripts read, through every API that reads it
#[test]
fn each_field_changes_what_scripts_read() {
    let dir = scratch("full-file");
    let file = dir.join("full.json");
    std::fs::write(&file, FULL).unwrap();
    let summary = run_harness("full", CLIENT_READS, &["--client-profile", &file.to_string_lossy()]);
    assert_eq!(summary["status"], "ok", "{summary}");
    assert_lines(
        &every_line(&summary),
        &[
            r#"print player Ada Ada L 5 400 Enum.MembershipType.Premium true de-de 9"#,
            r#"print inputs false false true false true true true"#,
            r#"print input Enum.PreferredInput.Touch Enum.UserInputType.Touch"#,
            r#"print gui true 0, 58 0, 0, 844, 58 Enum.PreferredTextSize.Large 0.5 true"#,
            r#"print screen 844, 390 0, 58, 844, 390"#,
            r#"print settings Enum.SavedQualitySetting.QualityLevel3 -1"#,
            r#"print vr true"#,
            r#"print locale de-de de-de DE"#,
            r#"print game 10 11 12 Enum.CreatorType.User job-1 3 ps 13 true"#,
            r#"print policy {"AllowedExternalLinkReferences":0,"AreAdsAllowed":false,"ArePaidRandomItemsRestricted":false,"IsContentSharingAllowed":true,"IsEligibleToPurchaseCommerceProduct":false,"IsEligibleToPurchaseSubscription":true,"IsEndlessContentAutoplayAllowed":true,"IsEndlessContentLoadAllowed":true,"IsPaidItemTradingAllowed":true,"IsPhotoToAvatarAllowed":false,"IsSubjectToChinaPolicies":false,"IsThirdPartyAccountLinkingAllowed":true}"#,
            r#"print owns true false true false true"#,
            r#"print owns async true"#,
            r#"print subscription {"IsSubscribed":true,"IsRenewing":true}"#,
            r#"print badges true false"#,
            r#"print badge list [41]"#,
            r#"print groups true 200 Admin false 0 Guest"#,
            r#"print group list [{"Id":7,"Name":"Seven","Rank":200,"Role":"Admin","IsPrimary":false}]"#,
            r#"print friends true true"#,
            r#"print online [{"VisitorId":2,"UserName":"Bo","DisplayName":"Bobo","IsOnline":true,"PlaceId":3}]"#,
            r#"print friend pages true"#,
            r#"print friend page [{"Id":2,"Username":"Bo","DisplayName":"Bobo","IsOnline":true},{"Id":3,"Username":"Cy","DisplayName":"Cy","IsOnline":false}]"#,
            r#"print names Ada Bo 5 2"#,
            r#"print infos [{"Id":5,"Username":"Ada","HasVerifiedBadge":true,"DisplayName":"Ada L"},{"Id":2,"Username":"Bo","HasVerifiedBadge":false,"DisplayName":"Bobo"}]"#,
            r#"print blocked [4,6]"#,
            r#"print player calls true 0.12"#,
            r#"print @done"#,
        ],
    );
    // (the summary says which profile: its layer, digest and fields)
    let profile = &summary["clientProfile"];
    assert_eq!(profile["sources"], serde_json::json!([file.to_string_lossy()]));
    assert_eq!(profile["profile"]["identity"]["name"], "Ada");
    assert_eq!(profile["digest"].as_str().unwrap().len(), 16);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The friends methods answer from the profile's friends for its player: IsFriendsWithAsync as IsFriendsWith (the id cast as a
/// client casts it), GetFriendsOnlineAsync as GetFriendsOnline (at most maxFriends entries), GetFriendsInServerAsync the
/// friends among the server's players (not the player), GetFriendsInUniverseAsync the online friends whose universeId is the
/// game's GameId, with their presence's PlaceId and GameId; another player's answers stay the offline ones
#[test]
fn friends_methods_answer_from_the_profiles_friends() {
    let profile = r#"{
	"identity": { "userId": 5 },
	"social": {
		"friends": [
			{ "userId": 2, "name": "Bo", "online": true, "universeId": 11, "presence": { "PlaceId": 3, "GameId": "job-2", "LocationType": 4 } },
			{ "userId": 3, "name": "Cy", "displayName": "Cyan", "online": true, "universeId": 12, "presence": { "PlaceId": 30 } },
			{ "userId": 6, "name": "Fi", "universeId": 11 }
		]
	},
	"game": { "gameId": 11 }
}"#;
    let harness = r#"return function()
	local Players = game:GetService("Players")
	local Http = game:GetService("HttpService")
	local P = Players.LocalPlayer
	local Friend = Instance.new("Player")
	Friend.UserId = 2
	Friend.Parent = Players
	local Stranger = Instance.new("Player")
	Stranger.UserId = 8
	Stranger.Parent = Players
	print("is", P:IsFriendsWithAsync(2), P:IsFriendsWithAsync(" 3 "), P:IsFriendsWithAsync(6), P:IsFriendsWithAsync(4), P:IsFriendsWith(2.9), P:IsFriendsWithAsync(-2))
	print("online", Http:JSONEncode(P:GetFriendsOnlineAsync()))
	print("online counts", #P:GetFriendsOnlineAsync(1), #P:GetFriendsOnline(0), #P:GetFriendsOnlineAsync(500))
	local InServer = P:GetFriendsInServerAsync()
	print("server", #InServer, InServer[1] == Friend)
	print("universe", Http:JSONEncode(P:GetFriendsInUniverseAsync()))
	print("other", Friend:IsFriendsWithAsync(5), #Friend:GetFriendsInServerAsync(), pcall(Friend.GetFriendsOnlineAsync, Friend))
end"#;
    let dir = scratch("friends");
    let file = dir.join("friends.json");
    std::fs::write(&file, profile).unwrap();
    let summary = run_harness("friends", harness, &["--client-profile", &file.to_string_lossy()]);
    assert_eq!(summary["status"], "ok", "{summary}");
    assert_lines(
        &printed(&summary),
        &[
            "is true true true false true false",
            r#"online [{"VisitorId":2,"UserName":"Bo","DisplayName":"Bo","IsOnline":true,"PlaceId":3,"GameId":"job-2","LocationType":4},{"VisitorId":3,"UserName":"Cy","DisplayName":"Cyan","IsOnline":true,"PlaceId":30}]"#,
            "online counts 1 0 2",
            "server 1 true",
            r#"universe [{"User":2,"PlaceId":3,"GameId":"job-2"}]"#,
            "other false 0 false GetFriendsOnline must be invoked on the Local Player",
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A preset changes what it says it does: the phone's inputs and screen, the console's ten-foot interface and gamepad, the
/// locale's ids and country, the China policy, the premium owner's membership and ownership
#[test]
fn presets_change_what_scripts_read() {
    let harness = r#"return function()
	local P = game:GetService("Players").LocalPlayer
	local UIS = game:GetService("UserInputService")
	local LS = game:GetService("LocalizationService")
	local Policy = game:GetService("PolicyService"):GetPolicyInfoForPlayerAsync(P)
	print(UIS.TouchEnabled, UIS.GamepadEnabled, UIS.KeyboardEnabled, UIS.PreferredInput.Name, game:GetService("GuiService"):IsTenFootInterface(), workspace.CurrentCamera.ViewportSize)
	print(P.LocaleId, LS.RobloxLocaleId, LS:GetCountryRegionForPlayerAsync(P), Policy.IsSubjectToChinaPolicies, #Policy.AllowedExternalLinkReferences)
	print(P.MembershipType.Name, P.AccountAge, game:GetService("MarketplaceService"):UserOwnsGamePassAsync(P.UserId, 123), P.Name, P.UserId)
end"#;
    let lines = |preset: &str| printed(&run_harness(preset, harness, &["--client-profile", preset]));
    assert_eq!(lines("phone")[0], "true false false Touch false 844, 390");
    assert_eq!(lines("console")[0], "false true false Gamepad true 1920, 1080");
    assert_eq!(lines("desktop-us")[..2], ["false false true KeyboardAndMouse false 1920, 1080", "en-us en-us US false 0"]);
    assert_eq!(lines("locale-de")[1], "de-de de-de DE false 0");
    assert_eq!(lines("china-policy")[1], "zh-cn zh-cn CN true 0");
    assert_eq!(lines("premium-owner")[2], "Premium 2000 true Player 1");
    assert_eq!(lines("new-account")[2], "None 0 false NewPlayer 9000000001");
}

/// The flags write their fields (and the profile's other fields stay); a flag and a --client-profile that disagree are refused
#[test]
fn flags_write_into_the_profile_and_conflicts_are_refused() {
    let harness = r#"return function()
	local P = game:GetService("Players").LocalPlayer
	print(P.Name, P.UserId, game.PlaceId, game:GetService("UserInputService").TouchEnabled, workspace.CurrentCamera.ViewportSize)
	print("@done")
end"#;
    let summary = run_harness("flags", harness, &["--client-profile", "phone", "--player", "Bob", "--place-id", "77", "--client-profile", r#"{"identity":{"userId":8}}"#]);
    assert_eq!(printed(&summary), ["Bob 8 77 true 844, 390", "@done"]);
    assert_eq!(summary["clientProfile"]["sources"], serde_json::json!(["phone", "inline"]));
    // (the same value is no conflict)
    let same = run_harness("flags-same", harness, &["--client-profile", r#"{"identity":{"name":"Bob"}}"#, "--player", "Bob"]);
    assert_eq!(printed(&same)[0], "Bob 1 0 false 1920, 1080");
    let refused = |extra: &[&str]| try_run("conflict", harness, extra).err().unwrap_or_default();
    assert_eq!(refused(&["--client-profile", r#"{"device":{"kind":"phone"}}"#, "--device", "desktop"]), "--device desktop and --client-profile (which sets device.kind to \"phone\") both set device.kind: pass one");
    // (a preset is a default: the flag goes over it)
    assert_eq!(printed(&run_harness("preset-device", harness, &["--client-profile", "phone", "--device", "desktop"]))[0], "Player 1 0 false 844, 390");
    assert_eq!(refused(&["--client-profile", r#"{"identity":{"name":"Ann"}}"#, "--player", "Bob"]), "--player Bob and --client-profile (which sets identity.name to \"Ann\") both set identity.name: pass one");
    assert!(refused(&["--client-profile", r#"{"identity":{"userId":"x"}}"#]).contains("identity.userId: expected a number, got \"x\""));
    assert!(refused(&["--client-profile", "no-such-preset"]).contains("the presets: default, desktop-us, phone, console"));
}

/// An edit run has a local player when its command line names one, through --player or a --client-profile
#[test]
fn an_edit_run_has_the_player_its_command_line_names() {
    let harness = r#"return function()
	local P = game:GetService("Players").LocalPlayer
	print(if P then P.Name .. " " .. P.UserId else "none")
end"#;
    let edit = |extra: &[&str]| {
        let mut words = vec!["--mode", "edit", "--run-context", "edit"];
        words.extend(extra);
        printed(&run_harness("edit", harness, &words))
    };
    assert_eq!(edit(&[]), ["none"]);
    assert_eq!(edit(&["--client-profile", r#"{"identity":{"userId":4}}"#]), ["none"], "a profile that names no player gives none");
    assert_eq!(edit(&["--client-profile", r#"{"identity":{"name":"Ed","userId":4}}"#]), ["Ed 4"]);
}

/// A server answers for the profile's player (its UserId) from the profile, for any other player as before; its data stores
/// and memory stores start with the profile's data
#[test]
fn a_server_answers_for_the_profiles_player_and_starts_with_its_data() {
    let profile = r#"{
		"identity": { "userId": 5 },
		"ownership": { "gamePasses": [11] },
		"policy": { "AreAdsAllowed": false },
		"social": { "groups": [{ "id": 7, "rank": 9, "role": "Mod" }] },
		"localization": { "countryRegion": "SE" },
		"data": {
			"dataStores": { "PlayerData": { "global": { "Player_5": { "Coins": 120, "Units": ["a", "b"] } }, "beta": { "k": 1 } } },
			"orderedDataStores": { "Wins": { "global": { "5": 30, "6": 10 } } },
			"globalDataStore": { "Motd": "hello" },
			"memoryStores": {
				"hashMaps": { "Lobby": { "open": true } },
				"sortedMaps": { "Ranks": { "a": { "value": 1, "sortKey": 2 }, "b": { "value": 3 } } },
				"queues": { "Jobs": ["x", "y"] }
			}
		}
	}"#;
    let summary = run_harness("server", include_str!("client_profile_tests/server.luau"), &["--run-context", "server", "--client-profile", profile]);
    assert_eq!(summary["status"], "ok", "{summary}");
    assert_eq!(
        printed(&summary),
        ["owns true false", "ads false true", "rank 9 0 Mod", "country SE US", "saved 120 2 1 nil", "wins 5 30 6 10", "global hello DataStoreKeyInfo", "lobby true", "ranks a 1 2 b", "jobs x,y", "@done"]
    );
}

/// A loopback pair: the client is the profile's player, and the server's Player for it has the same identity and answers
/// from the profile, with its data
#[test]
fn a_loopback_server_sees_the_joining_players_profile() {
    let profile = r#"{
		"identity": { "name": "Ada", "displayName": "Ada L", "userId": 5, "accountAge": 40, "membershipType": "Premium" },
		"localization": { "localeId": "fr-fr", "countryRegion": "FR" },
		"device": { "kind": "touch" },
		"ownership": { "gamePasses": [11] },
		"data": { "dataStores": { "PlayerData": { "global": { "Player_5": { "Coins": 7 } } } } }
	}"#;
    let summary = crate::loopback_tests::run_pair("profile", include_str!("client_profile_tests/loopback.luau"), None, &["--client-profile", profile]);
    assert_eq!(summary["status"], "ok", "{summary}");
    let lines = printed(&summary);
    for wanted in [
        "server: server Ada Ada L 5 40 Enum.MembershipType.Premium fr-fr",
        "server: server owns true FR",
        "server: server data 7",
        "client: client Ada Ada L 5 40 Enum.MembershipType.Premium fr-fr true",
    ] {
        assert!(lines.iter().any(|line| line == wanted), "{wanted} not in {lines:#?}");
    }
    assert_eq!(summary["clientProfile"]["sources"], serde_json::json!(["inline"]));
}

/// A harness reads the profile (Context.ClientProfile) and changes what is read at call time (Context.SetClientProfile): the
/// answers follow from then on, members it lets answer appear, and what was set up at the start is refused by name
#[test]
fn a_harness_reads_and_changes_the_profile_through_its_context() {
    let summary = run_harness("context", include_str!("client_profile_tests/context.luau"), &["--player", "Ann"]);
    assert_eq!(summary["status"], "ok", "{summary}");
    assert_eq!(
        printed(&summary),
        [
            r#"read {"identity":{"name":"Ann"}}"#,
            "before false true",
            "after true false true JP",
            r#"json false {"assets":"all"}"#,
            "refused false Context.SetClientProfile: identity is set when the run starts (pass it with --client-profile); a harness changes policy, ownership, social and localization.countryRegion",
            "bad false Context.SetClientProfile: ownership.gamePasses: expected a list of ids or \"all\", got \"some\"",
            "@done",
        ]
    );
    // (a flag alone is no profile layer: the summary names none)
    assert!(summary.get("clientProfile").is_none());
}

/// REE_CLIENT_PROFILE is the default profile, under the command line's layers; the shorthand variables sit between the two
#[test]
fn the_environment_gives_the_default_profile() {
    // (a child process: the variables are the process's)
    if std::env::var_os("REE_CLIENT_PROFILE_TEST_CHILD").is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args(["--exact", "client_profile_script_tests::the_environment_gives_the_default_profile", "--nocapture"]);
        child.env("REE_CLIENT_PROFILE_TEST_CHILD", "1").env("REE_CLIENT_PROFILE", "console").env("RUR_PLAYER", "FromEnv").env("RUR_PLAYER_ID", "12");
        child.env_remove("REE_DEVICE").env_remove("RUR_PLACE_ID").env_remove("RUR_CAMERA").env_remove("RUR_PLAYER_DISPLAY");
        let output = child.output().unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        return;
    }
    let harness = r#"return function()
	local P = game:GetService("Players").LocalPlayer
	print(P.Name, P.UserId, game:GetService("GuiService"):IsTenFootInterface(), game:GetService("UserInputService").GamepadEnabled)
end"#;
    let summary = run_harness("env", harness, &[]);
    assert_eq!(printed(&summary), ["FromEnv 12 true true"]);
    assert_eq!(summary["clientProfile"]["sources"], serde_json::json!(["console"]));
    // (the command line goes over the environment: its layer, then its flag)
    let over = run_harness("env-over", harness, &["--client-profile", r#"{"identity":{"userId":13},"device":{"tenFootInterface":false}}"#, "--player", "FromFlag"]);
    assert_eq!(printed(&over), ["FromFlag 13 false true"]);
    assert_eq!(over["clientProfile"]["sources"], serde_json::json!(["console", "inline"]));
}

/// A prepared run serves only the jobs with its client profile: the profile is in the key, and a profile file's change is
/// what makes a prepared run stale (it is noted as read)
#[test]
fn prepared_runs_are_keyed_by_the_profile() {
    let args = |profile: &str| -> Vec<String> { ["--harness", "h.luau", "--client-profile", profile, "--out", "o"].iter().map(|s| s.to_string()).collect() };
    let phone = crate::record::prepare::Keys::of(&args("phone"), false);
    let console = crate::record::prepare::Keys::of(&args("console"), false);
    assert_ne!(phone.full, console.full);
    assert!(crate::record::prepare::Key::of(&args("phone"), false, false).serves(&phone));
    assert!(!crate::record::prepare::Key::of(&args("phone"), false, false).serves(&console));
    // (a file layer is noted where a prepared run notes what it read)
    let dir = scratch("noted");
    let file = dir.join("p.json");
    std::fs::write(&file, "{}").unwrap();
    crate::files::start_noting();
    let mut layers = crate::client_profile::Layers::default();
    layers.add_spec(&file.to_string_lossy()).unwrap();
    let noted = crate::files::stop_noting();
    std::fs::write(&file, r#"{"identity":{"userId":2}}"#).unwrap();
    assert!(noted.changed().is_some(), "a changed profile file makes the prepared run stale");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--device` (and REE_DEVICE) now gives PreferredInput what a client of that device reports: Touch on a touch device or a
/// phone, Gamepad on a gamepad (the sim's 0.747 clients read the same); a desktop keeps KeyboardAndMouse. Main read
/// KeyboardAndMouse for every device: such runs are not byte-identical to main (review finding 1)
#[test]
fn a_device_flag_gives_its_preferred_input() {
    let harness = r#"return function()
	local UIS = game:GetService("UserInputService")
	print(UIS.PreferredInput.Name, UIS:GetLastInputType().Name, UIS.TouchEnabled, UIS.GamepadEnabled)
end"#;
    let device = |kind: &str| printed(&run_harness("device", harness, &["--device", kind]));
    assert_eq!(device("touch"), ["Touch Touch true false"]);
    assert_eq!(device("phone"), ["Touch Touch true false"]);
    assert_eq!(device("gamepad"), ["Gamepad Gamepad1 false true"]);
    assert_eq!(device("desktop"), ["KeyboardAndMouse MouseMovement false false"]);
}

/// A preset is a default the flags go over; a flag still may not disagree with a file or inline JSON (review finding 6)
#[test]
fn a_flag_goes_over_a_preset() {
    let harness = r#"return function()
	print(workspace.CurrentCamera.ViewportSize, game:GetService("UserInputService").TouchEnabled, game:GetService("Players").LocalPlayer.Name)
end"#;
    assert_eq!(printed(&run_harness("preset-flag", harness, &["--client-profile", "phone", "--viewport", "1170x2532"])), ["1170, 2532 true Player"]);
    assert_eq!(printed(&run_harness("preset-flag", harness, &["--client-profile", "new-account", "--player", "Flagged"])), ["1920, 1080 false Flagged"]);
    let refused = try_run("inline-flag", harness, &["--client-profile", r#"{"device":{"viewport":[844,390]}}"#, "--viewport", "1170x2532"]).err().unwrap_or_default();
    assert_eq!(refused, "--viewport 1170x2532 and --client-profile (which sets device.viewport to [844,390]) both set device.viewport: pass one");
}

/// Players:GetUserIdFromNameAsync knows the player's own name when the profile leaves it to the engine's "Player" (review
/// finding 4)
#[test]
fn the_players_default_name_finds_its_user_id() {
    let harness = r#"return function()
	local Players = game:GetService("Players")
	print(Players:GetUserIdFromNameAsync(Players.LocalPlayer.Name), Players:GetUserIdFromNameAsync("Someone"))
end"#;
    assert_eq!(printed(&run_harness("own-name", harness, &["--client-profile", r#"{"identity":{"userId":42}}"#])), ["42 1"]);
    assert_eq!(printed(&run_harness("own-name-flag", harness, &["--player", "Bob", "--client-profile", r#"{"identity":{"userId":43}}"#])), ["43 1"]);
}

/// A server answers the name lookups for the profile's player by its UserId, as its client does (review finding 5): a
/// loopback pair whose profile sets only the user id, and one that names the player
#[test]
fn a_server_looks_up_the_profiles_player_by_name() {
    let lines = |profile: &str| printed(&crate::loopback_tests::run_pair("names", include_str!("client_profile_tests/names.luau"), None, &["--client-profile", profile]));
    let id_only = lines(r#"{"identity":{"userId":5}}"#);
    assert!(id_only.contains(&"server: names Player Player Player false 5".to_string()), "{id_only:#?}");
    assert!(id_only.contains(&"client: names Player Player Player false 5".to_string()), "{id_only:#?}");
    let named = lines(r#"{"identity":{"userId":5,"name":"Ada","displayName":"Ada L","hasVerifiedBadge":true}}"#);
    assert!(named.contains(&"server: names Ada Ada Ada L true 5".to_string()), "{named:#?}");
    assert!(named.contains(&"client: names Ada Ada Ada L true 5".to_string()), "{named:#?}");
}

/// Context.SetClientProfile: a null that would remove a field set at the start is refused (review finding 2); removing
/// ownership leaves its members as with no profile, the ones it added reporting themselves again; an empty table is an
/// empty object or list, and Context.Null removes (review 7)
#[test]
fn a_harness_patch_removes_and_empties_as_json_does() {
    let summary = run_harness("patches", include_str!("client_profile_tests/patches.luau"), &["--client-profile", r#"{"localization":{"localeId":"de-de"},"ownership":{"assets":[3],"gamePasses":[11]}}"#]);
    assert_eq!(summary["status"], "ok", "{summary}");
    let lines = every_line(&summary);
    assert_eq!(
        lines,
        [
            "print null false Context.SetClientProfile: localization.localeId is set when the run starts (pass it with --client-profile); a harness changes policy, ownership, social and localization.countryRegion",
            "print owned true true",
            "missing MarketplaceService.PlayerOwnsAssetAsync is not emulated",
            "print removed false nil",
            // (an empty policy sets nothing: the profile has none)
            "print empty true [] null",
            "print listed true 0 []",
            "print @done",
        ]
    );
}

/// The engine Luau's lines as warnings' traces name them stay where main has them (string casts through services.luau's
/// StringParam, LogService's lines, the prelude's harness call): the profile's code adds no line above them
#[test]
fn engine_luau_trace_lines_stay_where_they_were() {
    let summary = run_harness("Traces", include_str!("client_profile_tests/traces.luau"), &[]);
    let traces: Vec<&str> = summary["output"].as_array().unwrap().iter().filter_map(|line| line["trace"].as_str()).collect();
    assert_eq!(
        traces,
        [
            "rur.services:32 function Read\nrur.services:1147 function RegistryName\nrur.services:1175\nReplicatedStorage.RURFixtures.Traces:3\nrur.record:1\nrur.prelude:680 function CallBody\n",
            "rur.services:32 function Read\nrur.services:1147 function RegistryName\nrur.services:1192\nReplicatedStorage.RURFixtures.Traces:4\nrur.record:1\nrur.prelude:680 function CallBody\n",
            "rur.services:32 function Read\nrur.services:1455 function LogLine\nrur.services:1509\nrur.services:45\nReplicatedStorage.RURFixtures.Traces:7\nReplicatedStorage.RURFixtures.Traces:6\nrur.record:1\nrur.prelude:680 function CallBody\n",
        ]
    );
}
