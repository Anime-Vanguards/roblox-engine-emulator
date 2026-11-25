//! The client profile's format: reading, checking, layering, presets and the schema file (client_profile.rs). What scripts
//! read of a profile is tested in tests.rs (client_profile_*), the command line's layers in launch.rs.

use serde_json::{Value as Json, json};

use super::*;

fn profile(value: Json) -> ClientProfile {
    ClientProfile::from_json(&value).unwrap_or_else(|error| panic!("{error}"))
}

fn refused(value: Json) -> String {
    ClientProfile::from_json(&value).err().unwrap_or_else(|| panic!("{value} was taken"))
}

/// The empty profile sets nothing and answers as the engine does without one
#[test]
fn the_empty_profile_is_the_engine_as_it_was() {
    let empty = profile(json!({}));
    assert!(empty.is_empty() && !empty.read_by_luau());
    assert_eq!(empty.to_json(), json!({}));
    assert_eq!((empty.identity.name(), empty.identity.display_name(), empty.identity.user_id()), ("Player", "Player", 1.0));
    assert_eq!((empty.device.kind(), empty.device.viewport(), empty.device.topbar_inset()), (DeviceKind::Desktop, (1920.0, 1080.0), 0.0));
    assert!(empty.device.keyboard_enabled() && empty.device.mouse_enabled() && !empty.device.touch_enabled() && !empty.device.gamepad_enabled());
    assert_eq!((empty.device.preferred_input(), empty.device.last_input_type(false), empty.device.last_input_type(true)), (None, "MouseMovement", "Focus"));
    assert_eq!((empty.localization.roblox_locale_id(), empty.localization.system_locale_id(), empty.localization.country_region()), ("en-us", "en-us", "US"));
    assert_eq!((empty.game.place_id(), empty.game.game_id()), (0.0, 0.0));
    assert_eq!(ClientProfile::default().to_json(), json!({}));
    assert_eq!(profile(json!({ "$schema": "../client-profile.schema.json" })).to_json(), json!({}));
}

/// Every field read back as written (sizes as [w, h], whole numbers as integers)
#[test]
fn every_field_reads_back() {
    let full = json!({
        "identity": { "name": "Ada", "displayName": "Ada L", "userId": 9000000001u64, "accountAge": 400, "membershipType": "Premium", "hasVerifiedBadge": true, "verified": true, "followUserId": 7 },
        "device": {
            "kind": "phone",
            "inputs": { "keyboard": true, "mouse": false, "touch": true, "gamepad": false, "accelerometer": true, "gyroscope": true, "vr": false },
            "preferredInput": "Touch", "lastInputType": "Touch", "tenFootInterface": false, "viewport": [844, 390], "camera": [844, 390], "topbarInset": 58, "networkPing": 0.12
        },
        "settings": { "preferredTextSize": "Large", "preferredTransparency": 0.5, "reducedMotion": true, "savedQualityLevel": "QualityLevel3", "cameraYInverted": true },
        "localization": { "localeId": "de-de", "robloxLocaleId": "de-de", "systemLocaleId": "de-de", "countryRegion": "DE" },
        "policy": { "AreAdsAllowed": false, "AllowedExternalLinkReferences": [] },
        "ownership": { "gamePasses": [1, 2], "assets": "all", "bundles": [], "badges": [5], "subscriptions": ["EXP-1"] },
        "social": { "groups": [{ "id": 7, "rank": 255, "role": "Owner", "name": "Seven", "isPrimary": true }], "friends": [{ "userId": 2, "name": "Bo", "online": true, "presence": { "PlaceId": 3 } }], "blocked": [4] },
        "game": { "placeId": 10, "gameId": 11, "creatorId": 12, "creatorType": "User", "jobId": "job", "placeVersion": 3, "privateServerId": "ps", "privateServerOwnerId": 13, "streamingTargetRadius": 400 },
        "data": {
            "dataStores": { "Players": { "global": { "Player_1": { "Coins": 5 } } } },
            "orderedDataStores": { "Wins": { "global": { "1": 3 } } },
            "globalDataStore": { "Motd": "hi" },
            "memoryStores": { "hashMaps": { "H": { "k": 1 } }, "sortedMaps": { "S": { "k": { "value": 1, "sortKey": 2 } } }, "queues": { "Q": [1, 2] } }
        }
    });
    let read = profile(full.clone());
    let mut expected = full;
    expected["device"]["topbarInset"] = json!(58.0);
    expected["game"]["streamingTargetRadius"] = json!(400.0);
    expected["device"]["viewport"] = json!([844.0, 390.0]);
    expected["device"]["camera"] = json!([844.0, 390.0]);
    assert_eq!(read.to_json(), expected);
    assert!(read.read_by_luau());
    assert_eq!(profile(read.to_json()).to_json(), read.to_json(), "to_json reads back the same");
    assert_eq!(read.identity.user_id(), 9000000001.0);
    assert_eq!(profile(json!({ "device": { "viewport": "800x600" } })).device.viewport(), (800.0, 600.0));
}

/// An error names the field: an unknown one with the fields there, a wrong type with what it got
#[test]
fn errors_name_the_field() {
    assert!(refused(json!({ "identty": {} })).starts_with("identty is not a client profile field (the fields there: identity, device,"));
    assert!(refused(json!({ "identity": { "userid": 1 } })).starts_with("identity.userid is not a client profile field"));
    assert_eq!(refused(json!({ "identity": { "userId": "x" } })), "identity.userId: expected a number, got \"x\"");
    assert_eq!(refused(json!({ "identity": { "userId": 1.5 } })), "identity.userId: expected a whole number, got 1.5");
    assert_eq!(refused(json!({ "identity": { "accountAge": -1 } })), "identity.accountAge: expected 0 or more, got -1");
    assert_eq!(refused(json!({ "identity": { "name": "" } })), "identity.name: a player's name is not empty");
    assert_eq!(refused(json!({ "identity": { "membershipType": "Gold" } })), "identity.membershipType: \"Gold\" is not an Enum.MembershipType item");
    assert_eq!(refused(json!({ "device": { "kind": "tablet" } })), "device.kind: expected desktop, touch, gamepad, phone, got \"tablet\"");
    assert!(refused(json!({ "device": { "inputs": { "pen": true } } })).starts_with("device.inputs.pen is not a client profile field"));
    assert_eq!(refused(json!({ "device": { "viewport": [0, 10] } })), "device.viewport: a size above 0, got 0 x 10");
    assert_eq!(refused(json!({ "device": { "viewport": "big" } })), "device.viewport: expected [width, height] or \"WxH\", got \"big\"");
    assert_eq!(refused(json!({ "device": { "preferredInput": "Mind" } })), "device.preferredInput: \"Mind\" is not an Enum.PreferredInput item");
    assert_eq!(refused(json!({ "settings": { "preferredTransparency": 2 } })), "settings.preferredTransparency: expected 0 to 1, got 2");
    assert_eq!(refused(json!({ "localization": { "countryRegion": "usa" } })), "localization.countryRegion: expected a two-letter country code like \"US\", got \"usa\"");
    assert_eq!(refused(json!({ "localization": { "localeId": "e" } })), "localization.localeId: expected a locale id like \"en-us\", got \"e\"");
    assert_eq!(refused(json!({ "policy": { "areAdsAllowed": true } })), "policy.areAdsAllowed: the policy table's keys are Roblox's (AreAdsAllowed, ...)");
    assert_eq!(refused(json!({ "policy": { "AllowedExternalLinkReferences": [1] } })), "policy.AllowedExternalLinkReferences: expected true, false, a number, a string or a list of strings, got a list");
    assert_eq!(refused(json!({ "ownership": { "gamePasses": "some" } })), "ownership.gamePasses: expected a list of ids or \"all\", got \"some\"");
    assert_eq!(refused(json!({ "ownership": { "badges": [1, "2"] } })), "ownership.badges[1]: expected a whole-number id, got \"2\"");
    assert_eq!(refused(json!({ "social": { "groups": [{ "id": 1, "rank": 0, "role": "Guest" }] } })), "social.groups[0].rank: expected 1 to 255 (a player in the group), got 0");
    assert_eq!(refused(json!({ "social": { "groups": [{ "id": 1, "rank": 1 }] } })), "social.groups[0].role: a group needs the player's role");
    assert_eq!(refused(json!({ "social": { "friends": [{ "userId": 2 }] } })), "social.friends[0].name: a friend needs a name");
    assert_eq!(refused(json!({ "social": { "blocked": ["x"] } })), "social.blocked[0]: expected a whole-number user id, got \"x\"");
    assert_eq!(refused(json!({ "game": { "creatorType": "Team" } })), "game.creatorType: \"Team\" is not an Enum.CreatorType item");
    assert_eq!(refused(json!({ "data": { "dataStores": { "P": { "global": { "k": null } } } } })), "data.dataStores.P.global.k: a data store holds no null");
    assert_eq!(refused(json!({ "data": { "orderedDataStores": { "W": { "global": { "k": 1.5 } } } } })), "data.orderedDataStores.W.global.k: an ordered data store holds whole numbers, got 1.5");
    assert_eq!(refused(json!({ "data": { "memoryStores": { "sortedMaps": { "S": { "k": { "sortKey": 1 } } } } } })), "data.memoryStores.sortedMaps.S.k.value: a sorted map's item needs its value");
    assert_eq!(refused(json!([])), "the profile: expected an object, got a list");
}

/// Layers merge as a JSON Merge Patch: objects key by key, lists replaced whole, null back to the engine's answer
#[test]
fn layers_merge_as_a_merge_patch() {
    let mut layers = Layers::default();
    layers.add(json!({ "identity": { "name": "Ada", "userId": 5 }, "ownership": { "gamePasses": [1, 2] }, "policy": { "AreAdsAllowed": false } }), "first").unwrap();
    layers.add(json!({ "identity": { "userId": 6 }, "ownership": { "gamePasses": [3] }, "policy": null }), "second").unwrap();
    let merged = layers.profile().unwrap();
    assert_eq!(merged.to_json(), json!({ "identity": { "name": "Ada", "userId": 6 }, "ownership": { "gamePasses": [3] } }));
    assert_eq!(merged.sources, ["first", "second"]);
    // (a bad layer is refused by its name, and leaves the layers as they were)
    let error = layers.add(json!({ "identity": { "userId": "x" } }), "third.json").unwrap_err();
    assert_eq!(error, "client profile third.json: identity.userId: expected a number, got \"x\"");
    assert_eq!(layers.sources(), ["first", "second"]);
    // (RFC 7396's own examples)
    let mut target = json!({ "a": "b", "c": { "d": "e", "f": "g" } });
    merge_patch(&mut target, &json!({ "a": "z", "c": { "f": null } }));
    assert_eq!(target, json!({ "a": "z", "c": { "d": "e" } }));
    let mut target = json!({ "a": [1, 2] });
    merge_patch(&mut target, &json!({ "a": [3] }));
    assert_eq!(target, json!({ "a": [3] }));
}

/// Every preset reads, and says what it is meant to
#[test]
fn presets_read() {
    for (name, _) in PRESETS {
        let value = preset(name).unwrap();
        ClientProfile::from_json(&value).unwrap_or_else(|error| panic!("preset {name}: {error}"));
    }
    let of = |name: &str| profile(preset(name).unwrap());
    assert!(of("default").is_empty());
    assert_eq!(of("phone").device.kind(), DeviceKind::Phone);
    assert!(of("phone").device.touch_enabled() && !of("phone").device.keyboard_enabled());
    let console = of("console");
    assert!(console.device.gamepad_enabled() && console.device.ten_foot_interface == Some(true) && !console.device.mouse_enabled());
    assert_eq!(of("locale-de").localization.country_region(), "DE");
    assert_eq!(of("china-policy").policy.as_ref().unwrap()["IsSubjectToChinaPolicies"], json!(true));
    assert_eq!(of("premium-owner").identity.membership_type.as_deref(), Some("Premium"));
    assert_eq!(of("premium-owner").ownership.as_ref().unwrap()["gamePasses"], json!("all"));
    assert_eq!(of("new-account").identity.account_age, Some(0.0));
    assert_eq!(of("desktop-us").localization.locale_id.as_deref(), Some("en-us"));
    let names: Vec<&str> = PRESETS.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["default", "desktop-us", "phone", "console", "locale-de", "china-policy", "premium-owner", "new-account"]);
}

/// A layer is a preset, inline JSON, or a file; several separated by `;`
#[test]
fn layers_are_presets_files_or_inline_json() {
    let folder = std::env::temp_dir().join(format!("ree-client-profile-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("me.json");
    std::fs::write(&file, r#"{ "identity": { "name": "FromFile" } }"#).unwrap();
    let mut layers = Layers::default();
    layers.add_spec(&format!("phone;{}", file.display())).unwrap();
    layers.add_spec(r#"{ "identity": { "userId": 3 } }"#).unwrap();
    let merged = layers.profile().unwrap();
    assert_eq!((merged.device.kind(), merged.identity.name(), merged.identity.user_id()), (DeviceKind::Phone, "FromFile", 3.0));
    assert_eq!(merged.sources, ["phone".to_string(), file.display().to_string(), "inline".into()]);
    let missing = Layers::default().add_spec("no-such-profile").unwrap_err();
    assert!(missing.starts_with("client profile no-such-profile: ") && missing.contains("the presets: default, desktop-us, phone"), "{missing}");
    std::fs::write(&file, "{ not json").unwrap();
    assert!(Layers::default().add_spec(&file.display().to_string()).unwrap_err().starts_with(&format!("client profile {}: ", file.display())));
    assert_eq!(absolute_spec("phone"), "phone");
    assert_eq!(absolute_spec(r#"{"a":1}"#), r#"{"a":1}"#);
    assert!(std::path::Path::new(absolute_spec("console;x.json").split_once(';').unwrap().1).is_absolute());
    let _ = std::fs::remove_dir_all(&folder);
}

/// What a running harness may change, and the error naming what it may not
#[test]
fn a_harness_changes_only_what_is_read_at_call_time() {
    assert!(live_patch(&json!({ "policy": { "AreAdsAllowed": false }, "ownership": { "gamePasses": [1] }, "social": { "blocked": [] }, "localization": { "countryRegion": "DE" } })).is_ok());
    assert_eq!(
        live_patch(&json!({ "identity": { "name": "X" } })).unwrap_err(),
        "identity is set when the run starts (pass it with --client-profile); a harness changes policy, ownership, social and localization.countryRegion"
    );
    assert!(live_patch(&json!({ "localization": { "localeId": "de-de" } })).unwrap_err().starts_with("localization.localeId is set when the run starts"));
    let base = profile(json!({ "identity": { "userId": 5 }, "ownership": { "gamePasses": [1] } }));
    let patched = base.patched(&json!({ "ownership": { "gamePasses": [1, 2] }, "policy": { "AreAdsAllowed": false } })).unwrap();
    assert_eq!(patched.to_json(), json!({ "identity": { "userId": 5 }, "ownership": { "gamePasses": [1, 2] }, "policy": { "AreAdsAllowed": false } }));
}

/// The digest follows the set fields and nothing else
#[test]
fn the_digest_follows_the_fields() {
    let a = profile(json!({ "device": { "kind": "phone" } }));
    let mut b = a.clone();
    b.sources = vec!["phone".into()];
    assert_eq!(a.digest(), b.digest());
    assert_ne!(a.digest(), profile(json!({ "device": { "kind": "touch" } })).digest());
    assert_eq!(a.digest().len(), 16);
    let summary = b.summary();
    assert_eq!((summary["sources"].clone(), summary["profile"].clone()), (json!(["phone"]), json!({ "device": { "kind": "phone" } })));
}

/// The schema file names every field the reader takes, and no other (data/client-profile.schema.json)
#[test]
fn the_schema_lists_every_field() {
    let schema: Json = serde_json::from_str(SCHEMA).unwrap();
    let properties = |value: &Json| -> Vec<String> { value["properties"].as_object().map(|map| map.keys().cloned().collect()).unwrap_or_default() };
    let mut listed = Vec::new();
    for section in properties(&schema).into_iter().filter(|key| key != "$schema") {
        let node = &schema["properties"][&section];
        for field in properties(node) {
            listed.push(format!("{section}.{field}"));
            let inner = &node["properties"][&field];
            let inner = if inner["items"].is_object() { &inner["items"] } else { inner };
            for sub in properties(inner) {
                listed.push(format!("{section}.{field}.{sub}"));
            }
        }
    }
    let mut known = fields();
    known.sort();
    listed.sort();
    assert_eq!(listed, known);
    let sections: Vec<String> = properties(&schema).into_iter().filter(|key| key != "$schema").collect();
    let mut expected: Vec<String> = SECTIONS.iter().map(|s| s.to_string()).collect();
    expected.sort();
    let mut sections = sections;
    sections.sort();
    assert_eq!(sections, expected);
}
