//! The client profile: everything about the emulated client and its player, in one place.
//!
//! A run's [`ClientProfile`] is the only source of what scripts read about who plays and on what: the player's identity
//! (Name, DisplayName, UserId, AccountAge, MembershipType ...), the device (its inputs, screen, topbar inset), the
//! player's settings, the locale and country, the policy answer, what the player owns, groups and friends, the game's ids
//! and the data stores' starting contents. Every reader in the engine reads it (docs/engine.md "Client profile" lists
//! them), and the command-line flags that set any of it (`--player`, `--device`, `--viewport`, `--camera`, `--place-id`,
//! `--streaming`) write into it.
//!
//! A profile is JSON ([`ClientProfile::from_json`]; the format's schema is `data/client-profile.schema.json`). Every field
//! is optional, and a field left unset keeps the engine's answer without a profile, so the empty profile is the engine as
//! it always was. Profiles are layered ([`Layers`]): built-in presets ([`PRESETS`]), files and inline JSON, each merged
//! over the ones before it as a JSON Merge Patch (RFC 7396): objects merge key by key, any other value (a list included)
//! replaces the one before, and `null` removes a field (back to the engine's own answer).

use std::sync::Arc;

use serde_json::{Map, Value as Json, json};

/// The built-in profiles (`--client-profile <name>`), as JSON: `data/client-profiles/<name>.json`.
pub const PRESETS: &[(&str, &str)] = &[
    ("default", include_str!("../data/client-profiles/default.json")),
    ("desktop-us", include_str!("../data/client-profiles/desktop-us.json")),
    ("phone", include_str!("../data/client-profiles/phone.json")),
    ("console", include_str!("../data/client-profiles/console.json")),
    ("locale-de", include_str!("../data/client-profiles/locale-de.json")),
    ("china-policy", include_str!("../data/client-profiles/china-policy.json")),
    ("premium-owner", include_str!("../data/client-profiles/premium-owner.json")),
    ("new-account", include_str!("../data/client-profiles/new-account.json")),
];

/// The JSON schema of the format (`data/client-profile.schema.json`)
pub const SCHEMA: &str = include_str!("../data/client-profile.schema.json");

/// The environment variable naming the default profile: layers as `--client-profile` takes them, separated by `;` (or one
/// inline JSON object). The command line's `--client-profile` layers go over it.
pub const ENV: &str = "REE_CLIENT_PROFILE";

/// What a client is played on: its inputs' defaults ([`Device`]), and for a phone the work clock's phone prices
/// (cost.rs `device_scale`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DeviceKind {
    #[default]
    Desktop,
    Touch,
    Gamepad,
    Phone,
}

impl DeviceKind {
    pub const NAMES: [&'static str; 4] = ["desktop", "touch", "gamepad", "phone"];

    pub fn parse(text: &str) -> Option<DeviceKind> {
        Some(match text {
            "desktop" => DeviceKind::Desktop,
            "touch" => DeviceKind::Touch,
            "gamepad" => DeviceKind::Gamepad,
            "phone" => DeviceKind::Phone,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            DeviceKind::Desktop => "desktop",
            DeviceKind::Touch => "touch",
            DeviceKind::Gamepad => "gamepad",
            DeviceKind::Phone => "phone",
        }
    }

    /// A touch screen (a phone is one)
    pub fn touch(self) -> bool {
        matches!(self, DeviceKind::Touch | DeviceKind::Phone)
    }
}

/// The player (`identity`): the local player's properties, and the server's Player for it in a loopback run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    /// Player.Name. Unset: "Player"
    pub name: Option<String>,
    /// Player.DisplayName. Unset: the name
    pub display_name: Option<String>,
    /// Player.UserId. Unset: 1
    pub user_id: Option<f64>,
    /// Player.AccountAge, in days. Unset: 0
    pub account_age: Option<f64>,
    /// Player.MembershipType, an Enum.MembershipType item's name. Unset: None
    pub membership_type: Option<String>,
    /// Player.HasVerifiedBadge (and UserService's HasVerifiedBadge for the player). Unset: false
    pub has_verified_badge: Option<bool>,
    /// Player:IsVerified(). Unset: false
    pub verified: Option<bool>,
    /// Player.FollowUserId. Unset: 0
    pub follow_user_id: Option<f64>,
}

impl Identity {
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("Player")
    }

    pub fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(self.name())
    }

    pub fn user_id(&self) -> f64 {
        self.user_id.unwrap_or(1.0)
    }
}

/// UserInputService's `*Enabled` inputs (`device.inputs`); each unset one follows the device's kind.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inputs {
    pub keyboard: Option<bool>,
    pub mouse: Option<bool>,
    pub touch: Option<bool>,
    pub gamepad: Option<bool>,
    pub accelerometer: Option<bool>,
    pub gyroscope: Option<bool>,
    /// UserInputService.VREnabled and VRService.VREnabled
    pub vr: Option<bool>,
}

/// The device (`device`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    /// desktop, touch, gamepad or phone. Unset: desktop
    pub kind: Option<DeviceKind>,
    pub inputs: Inputs,
    /// UserInputService.PreferredInput, an Enum.PreferredInput item's name. Unset: Touch on a touch device, Gamepad on a
    /// gamepad, else the class's default (KeyboardAndMouse)
    pub preferred_input: Option<String>,
    /// UserInputService:GetLastInputType(), an Enum.UserInputType item's name. Unset: Touch, Gamepad1, Focus in edit mode, else
    /// MouseMovement
    pub last_input_type: Option<String>,
    /// GuiService:IsTenFootInterface() (a console's). Unset: false
    pub ten_foot_interface: Option<bool>,
    /// The screen, in pixels (`--viewport`). Unset: 1920 x 1080
    pub viewport: Option<(f64, f64)>,
    /// The start camera's ViewportSize when it differs from the screen (`--camera`). Unset: the viewport
    pub camera: Option<(f64, f64)>,
    /// GuiService's topbar inset, in pixels (a GUI layer's, 58 in a client with Roblox's topbar). Unset: 0
    pub topbar_inset: Option<f64>,
    /// Player:GetNetworkPing(), in seconds. Unset: 0.05
    pub network_ping: Option<f64>,
}

impl Device {
    pub fn kind(&self) -> DeviceKind {
        self.kind.unwrap_or_default()
    }

    pub fn viewport(&self) -> (f64, f64) {
        self.viewport.unwrap_or((1920.0, 1080.0))
    }

    pub fn topbar_inset(&self) -> f64 {
        self.topbar_inset.unwrap_or(0.0)
    }

    pub fn touch_enabled(&self) -> bool {
        self.inputs.touch.unwrap_or(self.kind().touch())
    }

    pub fn keyboard_enabled(&self) -> bool {
        self.inputs.keyboard.unwrap_or(self.kind() == DeviceKind::Desktop)
    }

    pub fn mouse_enabled(&self) -> bool {
        self.inputs.mouse.unwrap_or(self.kind() == DeviceKind::Desktop)
    }

    pub fn gamepad_enabled(&self) -> bool {
        self.inputs.gamepad.unwrap_or(self.kind() == DeviceKind::Gamepad)
    }

    /// PreferredInput's item, None for the class's default
    pub fn preferred_input(&self) -> Option<&str> {
        self.preferred_input.as_deref().or(match self.kind() {
            DeviceKind::Touch | DeviceKind::Phone => Some("Touch"),
            DeviceKind::Gamepad => Some("Gamepad"),
            DeviceKind::Desktop => None,
        })
    }

    /// GetLastInputType's item in a run of `edit` mode or not
    pub fn last_input_type(&self, edit: bool) -> &str {
        if let Some(item) = &self.last_input_type {
            return item;
        }
        match self.kind() {
            DeviceKind::Touch | DeviceKind::Phone => "Touch",
            DeviceKind::Gamepad => "Gamepad1",
            // Measured: Studio in edit mode reports Focus (the window focus event)
            DeviceKind::Desktop if edit => "Focus",
            DeviceKind::Desktop => "MouseMovement",
        }
    }
}

/// The player's own settings (`settings`): what Roblox's settings menu and the device's accessibility options set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// GuiService.PreferredTextSize, an Enum.PreferredTextSize item's name
    pub preferred_text_size: Option<String>,
    /// GuiService.PreferredTransparency, 0 to 1
    pub preferred_transparency: Option<f64>,
    /// GuiService.ReducedMotionEnabled
    pub reduced_motion: Option<bool>,
    /// UserGameSettings.SavedQualityLevel, an Enum.SavedQualitySetting item's name
    pub saved_quality_level: Option<String>,
    /// UserGameSettings:GetCameraYInvertValue() reads -1
    pub camera_y_inverted: Option<bool>,
}

/// The locale and country (`localization`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Localization {
    /// Player.LocaleId (translators for the player use it). Unset: "" (the property's default)
    pub locale_id: Option<String>,
    /// LocalizationService.RobloxLocaleId. Unset: en-us
    pub roblox_locale_id: Option<String>,
    /// LocalizationService.SystemLocaleId. Unset: en-us
    pub system_locale_id: Option<String>,
    /// LocalizationService:GetCountryRegionForPlayerAsync for the player. Unset: US
    pub country_region: Option<String>,
}

/// The measured locale of a client's LocalizationService (RobloxLocaleId, SystemLocaleId: a 0.747 client and server alike)
pub const ROBLOX_LOCALE: &str = "en-us";

impl Localization {
    pub fn roblox_locale_id(&self) -> &str {
        self.roblox_locale_id.as_deref().unwrap_or(ROBLOX_LOCALE)
    }

    pub fn system_locale_id(&self) -> &str {
        self.system_locale_id.as_deref().unwrap_or(ROBLOX_LOCALE)
    }

    /// The country a client is in: no offline run has one, and "US" stands in
    pub fn country_region(&self) -> &str {
        self.country_region.as_deref().unwrap_or("US")
    }
}

/// The game the client joined (`game`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Game {
    /// game.PlaceId (`--place-id`). Unset: the project's (rur.json `placeIds`, a declared place id), else 0
    pub place_id: Option<f64>,
    /// game.GameId. Unset: 0
    pub game_id: Option<f64>,
    /// game.CreatorId. Unset: 0
    pub creator_id: Option<f64>,
    /// game.CreatorType, an Enum.CreatorType item's name. Unset: Group
    pub creator_type: Option<String>,
    /// game.JobId. Unset: ""
    pub job_id: Option<String>,
    /// game.PlaceVersion. Unset: 0
    pub place_version: Option<f64>,
    /// game.PrivateServerId. Unset: ""
    pub private_server_id: Option<String>,
    /// game.PrivateServerOwnerId. Unset: 0
    pub private_server_owner_id: Option<f64>,
    /// The place streams (Workspace.StreamingEnabled) with this StreamingTargetRadius (`--streaming`). Unset: the project's
    /// (rur.json `streamingTargetRadius`), else no streaming
    pub streaming_target_radius: Option<f64>,
}

impl Game {
    pub fn place_id(&self) -> f64 {
        self.place_id.unwrap_or(0.0)
    }

    pub fn game_id(&self) -> f64 {
        self.game_id.unwrap_or(0.0)
    }
}

/// The emulated client and player. Every field is optional; unset fields keep the engine's own answers.
#[derive(Clone, Debug, Default)]
pub struct ClientProfile {
    pub identity: Identity,
    pub device: Device,
    pub settings: Settings,
    pub localization: Localization,
    pub game: Game,
    /// PolicyService:GetPolicyInfoForPlayerAsync's table for the player: its keys go over the engine's answer
    pub policy: Option<Arc<Map<String, Json>>>,
    /// What the player owns, as the profile gives it (`gamePasses`, `assets`, `bundles`, `badges`: ids or "all";
    /// `subscriptions`: ids or "all")
    pub ownership: Option<Arc<Map<String, Json>>>,
    /// The player's groups, friends and blocked users, as the profile gives them
    pub social: Option<Arc<Map<String, Json>>>,
    /// The data stores' and memory stores' starting contents (a server's), as the profile gives them
    pub data: Option<Arc<Map<String, Json>>>,
    /// Where it came from, first to last: each layer's name (a preset's, a file's path, `inline`, a flag)
    pub sources: Vec<String>,
}

impl PartialEq for ClientProfile {
    fn eq(&self, other: &Self) -> bool {
        self.to_json() == other.to_json()
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading JSON, with errors that name the field

fn kind_of(value: &Json) -> String {
    match value {
        Json::Null => "null".into(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::String(text) => format!("{text:?}"),
        Json::Array(_) => "a list".into(),
        Json::Object(_) => "an object".into(),
    }
}

fn object<'a>(value: &'a Json, path: &str) -> Result<&'a Map<String, Json>, String> {
    value.as_object().ok_or_else(|| format!("{path}: expected an object, got {}", kind_of(value)))
}

/// `map`'s keys are all `fields` (else the first other one is named)
fn known(map: &Map<String, Json>, path: &str, fields: &[&str]) -> Result<(), String> {
    for key in map.keys() {
        if !fields.contains(&key.as_str()) {
            let at = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
            return Err(format!("{at} is not a client profile field (the fields there: {})", fields.join(", ")));
        }
    }
    Ok(())
}

fn at(path: &str, key: &str) -> String {
    if path.is_empty() { key.to_string() } else { format!("{path}.{key}") }
}

fn string(map: &Map<String, Json>, path: &str, key: &str) -> Result<Option<String>, String> {
    match map.get(key) {
        None => Ok(None),
        Some(Json::String(text)) => Ok(Some(text.clone())),
        Some(other) => Err(format!("{}: expected a string, got {}", at(path, key), kind_of(other))),
    }
}

fn boolean(map: &Map<String, Json>, path: &str, key: &str) -> Result<Option<bool>, String> {
    match map.get(key) {
        None => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(*flag)),
        Some(other) => Err(format!("{}: expected true or false, got {}", at(path, key), kind_of(other))),
    }
}

fn number(map: &Map<String, Json>, path: &str, key: &str) -> Result<Option<f64>, String> {
    match map.get(key) {
        None => Ok(None),
        Some(Json::Number(number)) => number.as_f64().map(Some).ok_or_else(|| format!("{}: not a number", at(path, key))),
        Some(other) => Err(format!("{}: expected a number, got {}", at(path, key), kind_of(other))),
    }
}

/// A whole number (an id, a count): any integer a double holds exactly
fn integer(map: &Map<String, Json>, path: &str, key: &str) -> Result<Option<f64>, String> {
    let value = number(map, path, key)?;
    if let Some(value) = value
        && (value.fract() != 0.0 || value.abs() > 9007199254740992.0)
    {
        return Err(format!("{}: expected a whole number, got {value}", at(path, key)));
    }
    Ok(value)
}

fn non_negative(map: &Map<String, Json>, path: &str, key: &str, whole: bool) -> Result<Option<f64>, String> {
    let value = if whole { integer(map, path, key)? } else { number(map, path, key)? };
    if let Some(value) = value
        && value < 0.0
    {
        return Err(format!("{}: expected 0 or more, got {value}", at(path, key)));
    }
    Ok(value)
}

/// A size: `[width, height]` or "WxH", each above 0
fn size(map: &Map<String, Json>, path: &str, key: &str) -> Result<Option<(f64, f64)>, String> {
    let name = at(path, key);
    let pair = match map.get(key) {
        None => return Ok(None),
        Some(Json::String(text)) => {
            let (w, h) = text.split_once(['x', 'X']).ok_or_else(|| format!("{name}: expected [width, height] or \"WxH\", got {text:?}"))?;
            let read = |part: &str| part.trim().parse::<f64>().map_err(|_| format!("{name}: expected [width, height] or \"WxH\", got {text:?}"));
            (read(w)?, read(h)?)
        }
        Some(Json::Array(items)) if items.len() == 2 && items.iter().all(Json::is_number) => (items[0].as_f64().unwrap_or(0.0), items[1].as_f64().unwrap_or(0.0)),
        Some(other) => return Err(format!("{name}: expected [width, height] or \"WxH\", got {}", kind_of(other))),
    };
    if !(pair.0 > 0.0 && pair.1 > 0.0 && pair.0.is_finite() && pair.1.is_finite()) {
        return Err(format!("{name}: a size above 0, got {} x {}", pair.0, pair.1));
    }
    Ok(Some(pair))
}

/// An Enum item's name: `enum_name`'s item (checked against the reflection's enums)
fn enum_item(map: &Map<String, Json>, path: &str, key: &str, enum_name: &str) -> Result<Option<String>, String> {
    let Some(name) = string(map, path, key)? else { return Ok(None) };
    let reflection = crate::reflection::Reflection::shared()?;
    if reflection.enum_item(enum_name, &name).is_none() {
        return Err(format!("{}: {name:?} is not an Enum.{enum_name} item", at(path, key)));
    }
    Ok(Some(name))
}

/// A locale id ("en-us"): letters, digits, `-` and `_`, 2 to 35 of them
fn locale(map: &Map<String, Json>, path: &str, key: &str) -> Result<Option<String>, String> {
    let Some(text) = string(map, path, key)? else { return Ok(None) };
    if text.len() < 2 || text.len() > 35 || !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(format!("{}: expected a locale id like \"en-us\", got {text:?}", at(path, key)));
    }
    Ok(Some(text))
}

/// A list of whole numbers (ids), or "all"
fn ids_or_all(value: &Json, name: &str) -> Result<(), String> {
    match value {
        Json::String(text) if text == "all" => Ok(()),
        Json::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                if !item.as_f64().is_some_and(|id| id.fract() == 0.0) {
                    return Err(format!("{name}[{index}]: expected a whole-number id, got {}", kind_of(item)));
                }
            }
            Ok(())
        }
        other => Err(format!("{name}: expected a list of ids or \"all\", got {}", kind_of(other))),
    }
}

/// JSON a data store can hold (no nulls in lists: a store's list has no holes)
fn storable(value: &Json, name: &str) -> Result<(), String> {
    match value {
        Json::Null => Err(format!("{name}: a data store holds no null")),
        Json::Array(items) => items.iter().enumerate().try_for_each(|(index, item)| storable(item, &format!("{name}[{index}]"))),
        Json::Object(map) => map.iter().try_for_each(|(key, item)| storable(item, &format!("{name}.{key}"))),
        _ => Ok(()),
    }
}

/// name -> (map of `inner`)
fn named<'a>(value: &'a Json, path: &str) -> Result<&'a Map<String, Json>, String> {
    let map = object(value, path)?;
    for name in map.keys() {
        if name.is_empty() || name.len() > 50 {
            return Err(format!("{path}: {name:?} is no store name (1 to 50 characters)"));
        }
    }
    Ok(map)
}

pub const SECTIONS: [&str; 9] = ["identity", "device", "settings", "localization", "policy", "ownership", "social", "game", "data"];
const IDENTITY: [&str; 8] = ["name", "displayName", "userId", "accountAge", "membershipType", "hasVerifiedBadge", "verified", "followUserId"];
const DEVICE: [&str; 9] = ["kind", "inputs", "preferredInput", "lastInputType", "tenFootInterface", "viewport", "camera", "topbarInset", "networkPing"];
const INPUTS: [&str; 7] = ["keyboard", "mouse", "touch", "gamepad", "accelerometer", "gyroscope", "vr"];
const SETTINGS: [&str; 5] = ["preferredTextSize", "preferredTransparency", "reducedMotion", "savedQualityLevel", "cameraYInverted"];
const LOCALIZATION: [&str; 4] = ["localeId", "robloxLocaleId", "systemLocaleId", "countryRegion"];
const OWNERSHIP: [&str; 5] = ["gamePasses", "assets", "bundles", "badges", "subscriptions"];
const SOCIAL: [&str; 3] = ["groups", "friends", "blocked"];
const GROUP: [&str; 5] = ["id", "rank", "role", "name", "isPrimary"];
const FRIEND: [&str; 5] = ["userId", "name", "displayName", "online", "presence"];
const GAME: [&str; 9] = ["placeId", "gameId", "creatorId", "creatorType", "jobId", "placeVersion", "privateServerId", "privateServerOwnerId", "streamingTargetRadius"];
const DATA: [&str; 4] = ["dataStores", "orderedDataStores", "globalDataStore", "memoryStores"];
const MEMORY: [&str; 3] = ["hashMaps", "sortedMaps", "queues"];

/// Every field of the format, as `section.field` (and `section.field.sub` inside device.inputs, social and data): what the
/// schema file lists (a test keeps the two the same)
pub fn fields() -> Vec<String> {
    let mut list = Vec::new();
    for (section, names) in [
        ("identity", &IDENTITY[..]),
        ("device", &DEVICE[..]),
        ("settings", &SETTINGS[..]),
        ("localization", &LOCALIZATION[..]),
        ("ownership", &OWNERSHIP[..]),
        ("social", &SOCIAL[..]),
        ("game", &GAME[..]),
        ("data", &DATA[..]),
    ] {
        list.extend(names.iter().map(|name| format!("{section}.{name}")));
    }
    list.extend(INPUTS.iter().map(|name| format!("device.inputs.{name}")));
    list.extend(GROUP.iter().map(|name| format!("social.groups.{name}")));
    list.extend(FRIEND.iter().map(|name| format!("social.friends.{name}")));
    list.extend(MEMORY.iter().map(|name| format!("data.memoryStores.{name}")));
    list
}

impl ClientProfile {
    /// A profile from its JSON (the format of `data/client-profile.schema.json`): an error names the field that is wrong.
    pub fn from_json(value: &Json) -> Result<ClientProfile, String> {
        let root = object(value, "the profile")?;
        let mut sections = root.clone();
        sections.remove("$schema");
        known(&sections, "", &SECTIONS)?;
        let mut profile = ClientProfile::default();
        if let Some(value) = root.get("identity") {
            let map = object(value, "identity")?;
            known(map, "identity", &IDENTITY)?;
            let path = "identity";
            let name = string(map, path, "name")?;
            if name.as_ref().is_some_and(|name| name.is_empty()) {
                return Err("identity.name: a player's name is not empty".into());
            }
            profile.identity = Identity {
                name,
                display_name: string(map, path, "displayName")?,
                user_id: integer(map, path, "userId")?,
                account_age: non_negative(map, path, "accountAge", true)?,
                membership_type: enum_item(map, path, "membershipType", "MembershipType")?,
                has_verified_badge: boolean(map, path, "hasVerifiedBadge")?,
                verified: boolean(map, path, "verified")?,
                follow_user_id: integer(map, path, "followUserId")?,
            };
        }
        if let Some(value) = root.get("device") {
            let map = object(value, "device")?;
            known(map, "device", &DEVICE)?;
            let path = "device";
            let kind = match string(map, path, "kind")? {
                None => None,
                Some(text) => Some(DeviceKind::parse(&text).ok_or_else(|| format!("device.kind: expected {}, got {text:?}", DeviceKind::NAMES.join(", ")))?),
            };
            let mut inputs = Inputs::default();
            if let Some(value) = map.get("inputs") {
                let map = object(value, "device.inputs")?;
                known(map, "device.inputs", &INPUTS)?;
                let path = "device.inputs";
                inputs = Inputs {
                    keyboard: boolean(map, path, "keyboard")?,
                    mouse: boolean(map, path, "mouse")?,
                    touch: boolean(map, path, "touch")?,
                    gamepad: boolean(map, path, "gamepad")?,
                    accelerometer: boolean(map, path, "accelerometer")?,
                    gyroscope: boolean(map, path, "gyroscope")?,
                    vr: boolean(map, path, "vr")?,
                };
            }
            profile.device = Device {
                kind,
                inputs,
                preferred_input: enum_item(map, path, "preferredInput", "PreferredInput")?,
                last_input_type: enum_item(map, path, "lastInputType", "UserInputType")?,
                ten_foot_interface: boolean(map, path, "tenFootInterface")?,
                viewport: size(map, path, "viewport")?,
                camera: size(map, path, "camera")?,
                topbar_inset: non_negative(map, path, "topbarInset", false)?,
                network_ping: non_negative(map, path, "networkPing", false)?,
            };
        }
        if let Some(value) = root.get("settings") {
            let map = object(value, "settings")?;
            known(map, "settings", &SETTINGS)?;
            let path = "settings";
            let transparency = number(map, path, "preferredTransparency")?;
            if let Some(value) = transparency
                && !(0.0..=1.0).contains(&value)
            {
                return Err(format!("settings.preferredTransparency: expected 0 to 1, got {value}"));
            }
            profile.settings = Settings {
                preferred_text_size: enum_item(map, path, "preferredTextSize", "PreferredTextSize")?,
                preferred_transparency: transparency,
                reduced_motion: boolean(map, path, "reducedMotion")?,
                saved_quality_level: enum_item(map, path, "savedQualityLevel", "SavedQualitySetting")?,
                camera_y_inverted: boolean(map, path, "cameraYInverted")?,
            };
        }
        if let Some(value) = root.get("localization") {
            let map = object(value, "localization")?;
            known(map, "localization", &LOCALIZATION)?;
            let path = "localization";
            let country = string(map, path, "countryRegion")?;
            if let Some(code) = &country
                && !(code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()))
            {
                return Err(format!("localization.countryRegion: expected a two-letter country code like \"US\", got {code:?}"));
            }
            profile.localization = Localization {
                locale_id: locale(map, path, "localeId")?,
                roblox_locale_id: locale(map, path, "robloxLocaleId")?,
                system_locale_id: locale(map, path, "systemLocaleId")?,
                country_region: country,
            };
        }
        if let Some(value) = root.get("policy") {
            let map = object(value, "policy")?;
            for (key, item) in map {
                if !key.bytes().next().is_some_and(|b| b.is_ascii_uppercase()) {
                    return Err(format!("policy.{key}: the policy table's keys are Roblox's (AreAdsAllowed, ...)"));
                }
                if item.is_null() || item.as_array().is_some_and(|items| items.iter().any(|item| !item.is_string())) {
                    return Err(format!("policy.{key}: expected true, false, a number, a string or a list of strings, got {}", kind_of(item)));
                }
            }
            profile.policy = Some(Arc::new(map.clone()));
        }
        if let Some(value) = root.get("ownership") {
            let map = object(value, "ownership")?;
            known(map, "ownership", &OWNERSHIP)?;
            for (key, item) in map {
                let name = format!("ownership.{key}");
                if key == "subscriptions" {
                    match item {
                        Json::String(text) if text == "all" => {}
                        Json::Array(items) if items.iter().all(|id| id.as_str().is_some_and(|id| !id.is_empty())) => {}
                        other => return Err(format!("{name}: expected a list of subscription ids (\"EXP-...\") or \"all\", got {}", kind_of(other))),
                    }
                } else {
                    ids_or_all(item, &name)?;
                }
            }
            profile.ownership = Some(Arc::new(map.clone()));
        }
        if let Some(value) = root.get("social") {
            let map = object(value, "social")?;
            known(map, "social", &SOCIAL)?;
            if let Some(groups) = map.get("groups") {
                let list = groups.as_array().ok_or_else(|| format!("social.groups: expected a list, got {}", kind_of(groups)))?;
                for (index, group) in list.iter().enumerate() {
                    let path = format!("social.groups[{index}]");
                    let group = object(group, &path)?;
                    known(group, &path, &GROUP)?;
                    if integer(group, &path, "id")?.is_none() {
                        return Err(format!("{path}.id: a group needs its id"));
                    }
                    let rank = integer(group, &path, "rank")?.ok_or_else(|| format!("{path}.rank: a group needs the player's rank (1 to 255)"))?;
                    if !(1.0..=255.0).contains(&rank) {
                        return Err(format!("{path}.rank: expected 1 to 255 (a player in the group), got {rank}"));
                    }
                    if string(group, &path, "role")?.is_none() {
                        return Err(format!("{path}.role: a group needs the player's role"));
                    }
                    string(group, &path, "name")?;
                    boolean(group, &path, "isPrimary")?;
                }
            }
            if let Some(friends) = map.get("friends") {
                let list = friends.as_array().ok_or_else(|| format!("social.friends: expected a list, got {}", kind_of(friends)))?;
                for (index, friend) in list.iter().enumerate() {
                    let path = format!("social.friends[{index}]");
                    let friend = object(friend, &path)?;
                    known(friend, &path, &FRIEND)?;
                    if integer(friend, &path, "userId")?.is_none() {
                        return Err(format!("{path}.userId: a friend needs a user id"));
                    }
                    if string(friend, &path, "name")?.is_none_or(|name| name.is_empty()) {
                        return Err(format!("{path}.name: a friend needs a name"));
                    }
                    string(friend, &path, "displayName")?;
                    boolean(friend, &path, "online")?;
                    if let Some(presence) = friend.get("presence") {
                        object(presence, &format!("{path}.presence"))?;
                    }
                }
            }
            if let Some(blocked) = map.get("blocked") {
                let Json::Array(items) = blocked else { return Err(format!("social.blocked: expected a list of user ids, got {}", kind_of(blocked))) };
                for (index, item) in items.iter().enumerate() {
                    if !item.as_f64().is_some_and(|id| id.fract() == 0.0) {
                        return Err(format!("social.blocked[{index}]: expected a whole-number user id, got {}", kind_of(item)));
                    }
                }
            }
            profile.social = Some(Arc::new(map.clone()));
        }
        if let Some(value) = root.get("game") {
            let map = object(value, "game")?;
            known(map, "game", &GAME)?;
            let path = "game";
            let radius = non_negative(map, path, "streamingTargetRadius", false)?;
            profile.game = Game {
                place_id: integer(map, path, "placeId")?,
                game_id: integer(map, path, "gameId")?,
                creator_id: integer(map, path, "creatorId")?,
                creator_type: enum_item(map, path, "creatorType", "CreatorType")?,
                job_id: string(map, path, "jobId")?,
                place_version: non_negative(map, path, "placeVersion", true)?,
                private_server_id: string(map, path, "privateServerId")?,
                private_server_owner_id: integer(map, path, "privateServerOwnerId")?,
                streaming_target_radius: radius,
            };
        }
        if let Some(value) = root.get("data") {
            let map = object(value, "data")?;
            known(map, "data", &DATA)?;
            for kind in ["dataStores", "orderedDataStores"] {
                let Some(stores) = map.get(kind) else { continue };
                let path = format!("data.{kind}");
                for (name, scopes) in named(stores, &path)? {
                    let path = format!("{path}.{name}");
                    for (scope, keys) in named(scopes, &path)? {
                        let path = format!("{path}.{scope}");
                        for (key, item) in named(keys, &path)? {
                            let path = format!("{path}.{key}");
                            if kind == "orderedDataStores" {
                                if !item.as_f64().is_some_and(|value| value.fract() == 0.0) {
                                    return Err(format!("{path}: an ordered data store holds whole numbers, got {}", kind_of(item)));
                                }
                            } else {
                                storable(item, &path)?;
                            }
                        }
                    }
                }
            }
            if let Some(keys) = map.get("globalDataStore") {
                for (key, item) in named(keys, "data.globalDataStore")? {
                    storable(item, &format!("data.globalDataStore.{key}"))?;
                }
            }
            if let Some(memory) = map.get("memoryStores") {
                let memory = object(memory, "data.memoryStores")?;
                known(memory, "data.memoryStores", &MEMORY)?;
                for kind in MEMORY {
                    let Some(structures) = memory.get(kind) else { continue };
                    let path = format!("data.memoryStores.{kind}");
                    for (name, contents) in named(structures, &path)? {
                        let path = format!("{path}.{name}");
                        match kind {
                            "queues" => {
                                let items = contents.as_array().ok_or_else(|| format!("{path}: a queue is a list of items, got {}", kind_of(contents)))?;
                                for (index, item) in items.iter().enumerate() {
                                    storable(item, &format!("{path}[{index}]"))?;
                                }
                            }
                            "sortedMaps" => {
                                for (key, entry) in named(contents, &path)? {
                                    let path = format!("{path}.{key}");
                                    let entry = object(entry, &path)?;
                                    known(entry, &path, &["value", "sortKey"])?;
                                    storable(entry.get("value").ok_or_else(|| format!("{path}.value: a sorted map's item needs its value"))?, &format!("{path}.value"))?;
                                    match entry.get("sortKey") {
                                        None | Some(Json::String(_)) | Some(Json::Number(_)) => {}
                                        Some(other) => return Err(format!("{path}.sortKey: expected a string or a number, got {}", kind_of(other))),
                                    }
                                }
                            }
                            _ => {
                                for (key, item) in named(contents, &path)? {
                                    storable(item, &format!("{path}.{key}"))?;
                                }
                            }
                        }
                    }
                }
            }
            profile.data = Some(Arc::new(map.clone()));
        }
        Ok(profile)
    }

    /// A profile from JSON text ([`ClientProfile::from_json`]): `ClientProfile::parse(r#"{"device": {"kind": "phone"}}"#)`
    pub fn parse(text: &str) -> Result<ClientProfile, String> {
        let value: Json = serde_json::from_str(text).map_err(|error| format!("client profile: {error}"))?;
        ClientProfile::from_json(&value)
    }

    /// The profile's set fields as JSON, in the format [`ClientProfile::from_json`] reads (the summary's and
    /// Context.ClientProfile's)
    pub fn to_json(&self) -> Json {
        fn put<T: serde::Serialize>(map: &mut Map<String, Json>, key: &str, value: &Option<T>) {
            if let Some(value) = value {
                map.insert(key.into(), json!(value));
            }
        }
        fn size(map: &mut Map<String, Json>, key: &str, value: Option<(f64, f64)>) {
            if let Some((w, h)) = value {
                map.insert(key.into(), json!([w, h]));
            }
        }
        // (whole numbers as integers: 1, not 1.0)
        fn whole(map: &mut Map<String, Json>, key: &str, value: Option<f64>) {
            if let Some(value) = value {
                map.insert(key.into(), if value.fract() == 0.0 && value.abs() < 9.007199254740992e15 { json!(value as i64) } else { json!(value) });
            }
        }
        let mut root = Map::new();
        let mut section = |name: &str, map: Map<String, Json>| {
            if !map.is_empty() {
                root.insert(name.into(), Json::Object(map));
            }
        };
        let i = &self.identity;
        let mut map = Map::new();
        put(&mut map, "name", &i.name);
        put(&mut map, "displayName", &i.display_name);
        whole(&mut map, "userId", i.user_id);
        whole(&mut map, "accountAge", i.account_age);
        put(&mut map, "membershipType", &i.membership_type);
        put(&mut map, "hasVerifiedBadge", &i.has_verified_badge);
        put(&mut map, "verified", &i.verified);
        whole(&mut map, "followUserId", i.follow_user_id);
        section("identity", map);
        let d = &self.device;
        let mut map = Map::new();
        put(&mut map, "kind", &d.kind.map(DeviceKind::name));
        let mut inputs = Map::new();
        let n = &d.inputs;
        for (key, value) in [("keyboard", n.keyboard), ("mouse", n.mouse), ("touch", n.touch), ("gamepad", n.gamepad), ("accelerometer", n.accelerometer), ("gyroscope", n.gyroscope), ("vr", n.vr)] {
            put(&mut inputs, key, &value);
        }
        if !inputs.is_empty() {
            map.insert("inputs".into(), Json::Object(inputs));
        }
        put(&mut map, "preferredInput", &d.preferred_input);
        put(&mut map, "lastInputType", &d.last_input_type);
        put(&mut map, "tenFootInterface", &d.ten_foot_interface);
        size(&mut map, "viewport", d.viewport);
        size(&mut map, "camera", d.camera);
        put(&mut map, "topbarInset", &d.topbar_inset);
        put(&mut map, "networkPing", &d.network_ping);
        section("device", map);
        let s = &self.settings;
        let mut map = Map::new();
        put(&mut map, "preferredTextSize", &s.preferred_text_size);
        put(&mut map, "preferredTransparency", &s.preferred_transparency);
        put(&mut map, "reducedMotion", &s.reduced_motion);
        put(&mut map, "savedQualityLevel", &s.saved_quality_level);
        put(&mut map, "cameraYInverted", &s.camera_y_inverted);
        section("settings", map);
        let l = &self.localization;
        let mut map = Map::new();
        put(&mut map, "localeId", &l.locale_id);
        put(&mut map, "robloxLocaleId", &l.roblox_locale_id);
        put(&mut map, "systemLocaleId", &l.system_locale_id);
        put(&mut map, "countryRegion", &l.country_region);
        section("localization", map);
        for (name, value) in [("policy", &self.policy), ("ownership", &self.ownership), ("social", &self.social)] {
            if let Some(value) = value {
                section(name, (**value).clone());
            }
        }
        let g = &self.game;
        let mut map = Map::new();
        whole(&mut map, "placeId", g.place_id);
        whole(&mut map, "gameId", g.game_id);
        whole(&mut map, "creatorId", g.creator_id);
        put(&mut map, "creatorType", &g.creator_type);
        put(&mut map, "jobId", &g.job_id);
        whole(&mut map, "placeVersion", g.place_version);
        put(&mut map, "privateServerId", &g.private_server_id);
        whole(&mut map, "privateServerOwnerId", g.private_server_owner_id);
        put(&mut map, "streamingTargetRadius", &g.streaming_target_radius);
        section("game", map);
        if let Some(data) = &self.data {
            section("data", (**data).clone());
        }
        Json::Object(root)
    }

    /// Whether no field is set (the engine's own answers throughout)
    pub fn is_empty(&self) -> bool {
        self.to_json().as_object().is_none_or(Map::is_empty)
    }

    /// A short digest of the set fields (FNV-1a over their JSON, keys sorted): two profiles with the same digest answer alike
    pub fn digest(&self) -> String {
        let text = serde_json::to_string(&self.to_json()).unwrap_or_default();
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }

    /// What a run's summary says of its profile (`clientProfile`): its layers, digest and set fields
    pub fn summary(&self) -> Json {
        json!({ "sources": self.sources, "digest": self.digest(), "profile": self.to_json() })
    }

    /// Whether the engine's Luau (services.luau, server.luau) reads any of the set fields: they then get the profile's
    /// JSON (R.Profile). A profile that sets none leaves that Luau as it was.
    pub fn read_by_luau(&self) -> bool {
        self.policy.is_some()
            || self.ownership.is_some()
            || self.social.is_some()
            || self.data.is_some()
            || self.identity.verified.is_some()
            || self.identity.has_verified_badge.is_some()
            || self.device.network_ping.is_some()
            || self.device.inputs.vr.is_some()
            || self.settings.camera_y_inverted.is_some()
    }

    /// `patch` merged over this profile ([`merge_patch`]), checked
    pub fn patched(&self, patch: &Json) -> Result<ClientProfile, String> {
        let mut merged = self.to_json();
        merge_patch(&mut merged, patch);
        let mut profile = ClientProfile::from_json(&merged)?;
        profile.sources = self.sources.clone();
        Ok(profile)
    }
}

/// RFC 7396 JSON Merge Patch: `patch` over `target` (objects merge key by key, a null removes a key, anything else
/// replaces)
pub fn merge_patch(target: &mut Json, patch: &Json) {
    let Json::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Json::Object(Map::new());
    }
    let Json::Object(map) = target else { return };
    for (key, value) in patch {
        if value.is_null() {
            map.remove(key);
        } else {
            merge_patch(map.entry(key.clone()).or_insert(Json::Null), value);
        }
    }
}

/// The sections a running harness may change (Context.SetClientProfile): what the engine reads at call time. The rest (the
/// player's properties, the device, the settings, the game and the stores' contents) was set up when the run started.
pub const LIVE_SECTIONS: [&str; 3] = ["policy", "ownership", "social"];

/// `patch` when it changes only what a running harness may change ([`LIVE_SECTIONS`] and localization.countryRegion); else
/// an error naming the first field it may not
pub fn live_patch(patch: &Json) -> Result<Json, String> {
    let map = object(patch, "the patch")?;
    for (key, value) in map {
        if LIVE_SECTIONS.contains(&key.as_str()) {
            continue;
        }
        if key == "localization"
            && let Some(fields) = value.as_object()
            && let Some(field) = fields.keys().find(|field| *field != "countryRegion")
        {
            return Err(format!("localization.{field} is set when the run starts (pass it with --client-profile); a harness changes policy, ownership, social and localization.countryRegion"));
        } else if key == "localization" {
            continue;
        }
        return Err(format!("{key} is set when the run starts (pass it with --client-profile); a harness changes policy, ownership, social and localization.countryRegion"));
    }
    Ok(patch.clone())
}

/// A preset's JSON by name
pub fn preset(name: &str) -> Option<Json> {
    PRESETS.iter().find(|(preset, _)| *preset == name).map(|(_, text)| serde_json::from_str(text).expect("a preset's JSON"))
}

/// One layer as `--client-profile` takes it: a preset's name, inline JSON (`{...}`), or a JSON file (any other text). Its
/// JSON and its name for [`ClientProfile::sources`] (a file's absolute path, as a service is given it); a file read is noted
/// for prepared runs (files.rs).
pub fn read_layer(spec: &str) -> Result<(Json, String), String> {
    let spec = spec.trim();
    if spec.starts_with('{') {
        let value = serde_json::from_str(spec).map_err(|error| format!("client profile (inline JSON): {error}"))?;
        return Ok((value, "inline".into()));
    }
    if let Some(value) = preset(spec) {
        return Ok((value, spec.to_string()));
    }
    let path = std::path::Path::new(spec);
    crate::files::note(path);
    let text = std::fs::read_to_string(path).map_err(|error| {
        let presets: Vec<&str> = PRESETS.iter().map(|(name, _)| *name).collect();
        format!("client profile {spec}: {error} (a profile is a JSON file, inline JSON, or one of the presets: {})", presets.join(", "))
    })?;
    let value = serde_json::from_str(&text).map_err(|error| format!("client profile {spec}: {error}"))?;
    Ok((value, crate::launch::absolute(spec)))
}

/// The layers `spec` names, each read once ([`read_layer`]; several separated by `;` unless it is inline JSON)
pub fn read_spec(spec: &str) -> Result<Vec<(Json, String)>, String> {
    let spec = spec.trim();
    let specs: Vec<&str> = if spec.starts_with('{') { vec![spec] } else { spec.split(';').map(str::trim).filter(|part| !part.is_empty()).collect() };
    specs.into_iter().map(read_layer).collect()
}

/// `spec` (layers as `--client-profile` and [`ENV`] take them) with each file made absolute from the current directory:
/// what a service or a worker, which runs in another folder, is given. Presets and inline JSON stay as they are.
pub fn absolute_spec(spec: &str) -> String {
    let trimmed = spec.trim();
    if trimmed.starts_with('{') {
        return spec.to_string();
    }
    trimmed
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| if preset(part).is_some() { part.to_string() } else { crate::launch::absolute(part) })
        .collect::<Vec<_>>()
        .join(";")
}

/// Layers of a client profile, merged in the order they are added (later over earlier); each is checked as it comes, so an
/// error names its layer.
#[derive(Clone, Debug, Default)]
pub struct Layers {
    merged: Json,
    sources: Vec<String>,
}

impl Layers {
    /// Adds `value`, named `source` (its nulls remove fields: what is left of it is checked)
    pub fn add(&mut self, value: Json, source: &str) -> Result<(), String> {
        ClientProfile::from_json(&without_nulls(&value)).map_err(|error| format!("client profile {source}: {error}"))?;
        if self.merged.is_null() {
            self.merged = Json::Object(Map::new());
        }
        merge_patch(&mut self.merged, &value);
        self.sources.push(source.to_string());
        Ok(())
    }

    /// Adds the layers `spec` names ([`read_spec`])
    pub fn add_spec(&mut self, spec: &str) -> Result<(), String> {
        for (value, source) in read_spec(spec)? {
            self.add(value, &source)?;
        }
        Ok(())
    }

    /// The JSON merged so far (an empty object when nothing was added)
    pub fn json(&self) -> Json {
        if self.merged.is_null() { Json::Object(Map::new()) } else { self.merged.clone() }
    }

    pub fn sources(&self) -> &[String] {
        &self.sources
    }

    /// The profile of the layers
    pub fn profile(&self) -> Result<ClientProfile, String> {
        let mut profile = ClientProfile::from_json(&self.json())?;
        profile.sources = self.sources.clone();
        Ok(profile)
    }
}

/// `value` without the keys a merge patch removes (those whose value is null), at any depth
fn without_nulls(value: &Json) -> Json {
    match value {
        Json::Object(map) => Json::Object(map.iter().filter(|(_, item)| !item.is_null()).map(|(key, item)| (key.clone(), without_nulls(item))).collect()),
        other => other.clone(),
    }
}

/// The value at `path` (dot-separated) in `value`, if set
pub fn get<'a>(value: &'a Json, path: &str) -> Option<&'a Json> {
    path.split('.').try_fold(value, |value, key| value.get(key))
}

#[cfg(test)]
#[path = "client_profile_tests.rs"]
mod tests;
