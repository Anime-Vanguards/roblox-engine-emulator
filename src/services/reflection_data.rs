//! What ReflectionService answers from: every class and member a client's ReflectionService knows, with the capabilities each
//! needs, read once a process (a class's members when a query first needs them) from `data/reflection.json` (a game script's default view) and `data/reflection-security.json`
//! (what a Security filter adds: tools/reflection-security.mjs). The engine reads none of it; services/reflection_service.rs
//! builds the answers.
//!
//! Measured in roblox-sim's 0.747 playing client and server (identity 2; docs/engine.md "ReflectionService"):
//! - a filter's Security is a set of capabilities, a game script's own (SecurityCapabilities.fromCurrent) when absent; Unassigned
//!   counts as held whatever the set;
//! - GetClass and GetClasses list a class whose `list` capabilities the set holds (a class's New or GetService permit, else its
//!   own); the member queries answer for a class whose `members` capabilities it holds (the same but for Player, NetworkClient
//!   and NetworkServer, whose New and GetService need one more), and for the settings classes GetClasses never lists;
//! - a property is listed when the set holds its Read permit's capabilities (one with no Read permit: its own), every method
//!   and event always; a member a subclass declares again (ValueBase classes' Changed, Workspace:BreakJoints) is listed once,
//!   the subclass's;
//! - a Permits table holds the actions whose capabilities the set holds, each a SecurityCapabilities value.

use std::sync::OnceLock;

use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;

/// The capabilities in the order SecurityCapabilities prints them (src/lua/prelude.luau CapabilityOrder, which a test
/// compares with this): a set's bit `n` is the `n`th of them.
pub(crate) const CAPABILITY_ORDER: [&str; 51] = [
    "Plugin", "LocalUser", "WritePlayer", "RobloxScript", "RobloxEngine", "RunClientScript", "RunServerScript", "AccessOutsideWrite",
    "Unassigned", "LoadUnownedAsset", "LoadString", "ScriptGlobals", "CreateInstances", "Basic", "Audio", "DataStore", "Network",
    "Physics", "UI", "CSG", "Chat", "Animation", "AvatarAppearance", "Input", "Environment", "RemoteEvent", "LegacySound",
    "Players", "CapabilityControl", "AssetRead", "AssetManagement", "DynamicGeneration", "PlatformAvatarEditing",
    "AssetCreateUpdate", "Capture", "SensitiveInput", "Monetization", "LoadOwnedAsset", "Social", "ServerCommunication",
    "Logging", "PromptExternalPurchase", "Groups", "Teleport", "Consequences", "Material", "AvatarBehavior", "RemoteCommand",
    "InternalTest", "PluginOrOpenCloud", "Assistant",
];

/// A set of capabilities: bit `n` is [`CAPABILITY_ORDER`]'s `n`th.
pub(crate) type Caps = u64;

/// Unassigned's bit: a requirement never needs it (Debris.MaxItems's Write permit names it and shows whatever the set holds)
pub(crate) const UNASSIGNED: Caps = 1 << 8;

/// A game script's set (SecurityCapabilities.fromCurrent, the same on a client and a server): RunClientScript to
/// AvatarBehavior, every capability but the first five and the last four.
pub(crate) const SCRIPT_CAPS: Caps = ((1 << 47) - 1) & !((1 << 5) - 1);

/// A plugin's set, what ree's edit run context holds: a game script's and Plugin and PluginOrOpenCloud. A plugin's code (Studio 0.748, edit
/// mode) is refused what needs RobloxScript, RobloxEngine or LocalUser (reflection.rs CAPABILITY_CALLS). The command bar's identity 4
/// holds LocalUser too: a playing client's identity-4 job reads DataCost and RobloxLocked, writes Lighting.LightingStyle, and is refused
/// what needs RobloxScript, RobloxEngine or InternalTest (roblox-sim 0.747, every property data/capability-properties.rs lists that the
/// job reaches); SecurityCapabilities.fromCurrent() prints a game script's set at identities 2, 4 and 8 alike.
pub(crate) const PLUGIN_CAPS: Caps = SCRIPT_CAPS | 1 | (1 << 49);

/// Whether `held` holds every capability of `need` (Unassigned is always held)
pub(crate) fn holds(held: Caps, need: Caps) -> bool {
    need & !(held | UNASSIGNED) == 0
}

/// The capability of `need` that `held` lacks, the first in [`CAPABILITY_ORDER`] when it lacks several (a client's error names one;
/// which of several is not measured: no permit a game script can reach needs two it lacks); None when it holds them all.
pub(crate) fn first_lacking(held: Caps, need: Caps) -> Option<&'static str> {
    let missing = need & !(held | UNASSIGNED);
    (missing != 0).then(|| CAPABILITY_ORDER[missing.trailing_zeros() as usize])
}

/// The set a permit's text names ("Basic | UI"; "" none). A name not in [`CAPABILITY_ORDER`] (the set's other names for two,
/// and anything newer) is read as its alias or left out.
pub(crate) fn caps_of(text: &str) -> Caps {
    let mut caps = 0;
    for name in text.split('|').map(str::trim).filter(|name| !name.is_empty()) {
        let name = match name {
            "AssetRequire" => "LoadUnownedAsset",
            "Avatar" => "AvatarAppearance",
            other => other,
        };
        if let Some(at) = CAPABILITY_ORDER.iter().position(|known| *known == name) {
            caps |= 1 << at;
        }
    }
    caps
}

/// An action a Permits table names
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Action(pub &'static str);

const ACTIONS: &[&str] = &["New", "GetService", "Read", "Write", "ReadParallel", "WriteParallel", "Call", "CallParallel", "Listen", "ListenParallel"];

fn action(name: &str) -> Option<Action> {
    ACTIONS.iter().find(|known| **known == name).map(|known| Action(known))
}

/// A member's or class's permits: each action a script may take and the capabilities it needs
pub(crate) type Permits = Box<[(Action, Caps)]>;

/// Display data: Category and, when deprecated, DeprecationMessage
pub(crate) type Display = Box<[(Box<str>, Box<str>)]>;

/// A ReflectionType: the keys its table has
#[derive(Debug, Default)]
pub(crate) struct Type {
    pub script: Option<Box<str>>,
    pub engine: Option<Box<str>>,
    pub enum_name: Option<Box<str>>,
    pub instance: Option<Box<str>>,
}

#[derive(Debug)]
pub(crate) struct Parameter {
    pub name: Box<str>,
    pub ty: Type,
    /// Its DefaultValue as the dump encodes it (crate::value::decode_default reads it), when it has one
    pub default: Option<serde_json::Value>,
}

#[derive(Debug)]
pub(crate) struct Property {
    pub name: Box<str>,
    pub ty: Type,
    pub serialized: bool,
    /// ContentType: an Enum.AssetType item's name
    pub content: Option<Box<str>>,
    pub permits: Permits,
    pub display: Display,
    /// What listing it needs
    pub visible: Caps,
}

#[derive(Debug)]
pub(crate) struct Method {
    pub name: Box<str>,
    pub returns: Type,
    pub parameters: Box<[Parameter]>,
    pub permits: Permits,
    pub display: Display,
}

#[derive(Debug)]
pub(crate) struct Event {
    pub name: Box<str>,
    pub parameters: Box<[Parameter]>,
    pub permits: Permits,
    pub display: Display,
}

pub(crate) struct Class {
    pub name: Box<str>,
    pub superclass: Option<u32>,
    pub serialized: bool,
    pub display: Display,
    pub permits: Permits,
    /// GetClass and GetClasses can give it (false for the settings classes and a name only a Subclasses list has)
    pub listed: bool,
    /// The member queries answer for it (false for a name only a Subclasses list has)
    pub answers: bool,
    /// What GetClass and GetClasses need to give it
    pub list: Caps,
    /// What the member queries need to answer for it
    pub members: Caps,
    /// Every class whose superclass it is, in name order
    pub subclasses: Box<[u32]>,
    /// Its object in data/reflection.json, and its entry in data/reflection-security.json (`classes` or `extend`)
    dump: Option<&'static RawValue>,
    security: Option<&'static RawValue>,
    /// Its own members, read from those on the first query that needs them
    own: OnceLock<Members>,
}

/// A class's own members, in the client's order
#[derive(Debug, Default)]
pub(crate) struct Members {
    pub properties: Box<[Property]>,
    pub methods: Box<[Method]>,
    pub events: Box<[Event]>,
}

pub(crate) struct Data {
    /// In name order
    pub classes: Box<[Class]>,
    by_name: FxHashMap<Box<str>, u32>,
}

impl Data {
    pub(crate) fn index(&self, name: &str) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    /// Whether `class` is `ancestor` or one of its subclasses
    pub(crate) fn is_a(&self, mut class: u32, ancestor: u32) -> bool {
        loop {
            if class == ancestor {
                return true;
            }
            match self.classes[class as usize].superclass {
                Some(superclass) => class = superclass,
                None => return false,
            }
        }
    }

    /// The class's own members (read on first use)
    pub(crate) fn members(&self, class: u32) -> &Members {
        let class = &self.classes[class as usize];
        class.own.get_or_init(|| members_of(class))
    }
}

static DATA: OnceLock<Data> = OnceLock::new();

/// The classes, read on first use (a few milliseconds; their members wait for a query that needs them).
pub(crate) fn data() -> &'static Data {
    DATA.get_or_init(|| build(crate::data::REFLECTION, crate::data::REFLECTION_SECURITY))
}

// The files' shapes. An empty table is `[]` in data/reflection.json (Studio's JSONEncode) and `{}` in the other.

#[derive(Default)]
struct Pairs(Vec<(String, String)>);

impl<'de> Deserialize<'de> for Pairs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Pairs, D::Error> {
        struct PairsVisitor;
        impl<'de> Visitor<'de> for PairsVisitor {
            type Value = Pairs;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a map of texts or an empty list")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Pairs, A::Error> {
                let mut pairs = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    pairs.push((key, value));
                }
                Ok(Pairs(pairs))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Pairs, A::Error> {
                while seq.next_element::<de::IgnoredAny>()?.is_some() {}
                Ok(Pairs(Vec::new()))
            }
        }
        deserializer.deserialize_any(PairsVisitor)
    }
}

#[derive(Deserialize, Default)]
struct FileType {
    script: Option<String>,
    engine: Option<String>,
    #[serde(rename = "enum")]
    enum_name: Option<String>,
    instance: Option<String>,
}

#[derive(Deserialize)]
struct FileParameter {
    name: String,
    #[serde(rename = "type", default)]
    ty: FileType,
    default: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct FileProperty {
    name: String,
    #[serde(rename = "type", default)]
    ty: FileType,
    #[serde(default)]
    serialized: bool,
    content: Option<String>,
    #[serde(default)]
    permits: Pairs,
    #[serde(default)]
    display: Pairs,
    visible: Option<String>,
    at: Option<usize>,
}

#[derive(Deserialize)]
struct FileMethod {
    name: String,
    returns: Option<FileType>,
    #[serde(default)]
    parameters: Vec<FileParameter>,
    #[serde(default)]
    permits: Pairs,
    #[serde(default)]
    display: Pairs,
}

#[derive(Deserialize)]
struct FileEvent {
    name: String,
    #[serde(default)]
    parameters: Vec<FileParameter>,
    #[serde(default)]
    permits: Pairs,
    #[serde(default)]
    display: Pairs,
}

/// A class's own fields (its members stay in the text until a query needs them: [`FileMembers`])
#[derive(Deserialize)]
struct FileClass {
    name: String,
    superclass: Option<String>,
    serialized: Option<bool>,
    #[serde(default)]
    permits: Pairs,
    #[serde(default)]
    display: Pairs,
    // (data/reflection-security.json's)
    listed: Option<bool>,
    #[serde(default)]
    subclass: bool,
    list: Option<String>,
    members: Option<String>,
}

#[derive(Deserialize)]
struct FileMembers {
    #[serde(default)]
    properties: Vec<FileProperty>,
    #[serde(default)]
    methods: Vec<FileMethod>,
    #[serde(default)]
    events: Vec<FileEvent>,
}

#[derive(Deserialize, Default)]
struct FilePermitsByKind {
    #[serde(default)]
    properties: FxHashMap<String, Pairs>,
    #[serde(default)]
    methods: FxHashMap<String, Pairs>,
    #[serde(default)]
    events: FxHashMap<String, Pairs>,
}

#[derive(Deserialize)]
struct FileExtendHead {
    name: String,
    list: Option<String>,
}

#[derive(Deserialize)]
struct FileExtend {
    #[serde(default)]
    properties: Vec<FileProperty>,
    #[serde(default)]
    permits: FilePermitsByKind,
    #[serde(default)]
    visible: FxHashMap<String, String>,
}

#[derive(Deserialize)]
struct DumpFile {
    #[serde(borrow)]
    classes: Vec<&'static RawValue>,
}

#[derive(Deserialize)]
struct SecurityFile {
    #[serde(borrow)]
    classes: Vec<&'static RawValue>,
    #[serde(borrow)]
    extend: Vec<&'static RawValue>,
}

fn permits(pairs: &Pairs) -> Permits {
    pairs.0.iter().filter_map(|(name, text)| Some((action(name)?, caps_of(text)))).collect()
}

fn display(pairs: &Pairs) -> Display {
    pairs.0.iter().map(|(key, text)| (key.as_str().into(), text.as_str().into())).collect()
}

fn ty(file: &FileType) -> Type {
    Type {
        script: file.script.as_deref().map(Into::into),
        engine: file.engine.as_deref().map(Into::into),
        enum_name: file.enum_name.as_deref().map(Into::into),
        instance: file.instance.as_deref().map(Into::into),
    }
}

fn parameters(list: Vec<FileParameter>) -> Box<[Parameter]> {
    list.into_iter().map(|parameter| Parameter { name: parameter.name.into(), ty: ty(&parameter.ty), default: parameter.default }).collect()
}

fn property(file: FileProperty) -> Property {
    let permits = permits(&file.permits);
    // (a Read permit says what listing it needs; a property with none names its own)
    let visible = match permits.iter().find(|(action, _)| action.0 == "Read") {
        Some(&(_, caps)) => caps,
        None => file.visible.as_deref().map(caps_of).unwrap_or(0),
    };
    Property { name: file.name.into(), ty: ty(&file.ty), serialized: file.serialized, content: file.content.map(Into::into), permits, display: display(&file.display), visible }
}

fn method(file: FileMethod) -> Method {
    Method {
        name: file.name.into(),
        returns: file.returns.as_ref().map(ty).unwrap_or_default(),
        parameters: parameters(file.parameters),
        permits: permits(&file.permits),
        display: display(&file.display),
    }
}

fn event(file: FileEvent) -> Event {
    Event { name: file.name.into(), parameters: parameters(file.parameters), permits: permits(&file.permits), display: display(&file.display) }
}

/// A class's list capabilities: those of its New or GetService permit, else its own
fn list_caps(permits: &Permits, own: Option<&str>) -> Caps {
    permits.iter().find(|(action, _)| action.0 == "New" || action.0 == "GetService").map(|&(_, caps)| caps).unwrap_or_else(|| own.map(caps_of).unwrap_or(0))
}

fn add_permits(target: &mut Permits, more: &Pairs) {
    let mut list: Vec<(Action, Caps)> = target.to_vec();
    list.extend(permits(more).iter().copied());
    *target = list.into_boxed_slice();
}

fn read<T: Deserialize<'static>>(raw: &'static RawValue) -> T {
    serde_json::from_str(raw.get()).expect("ReflectionService's data reads")
}

/// A class's own members: data/reflection.json's with the hidden ones and permits of its `extend` entry, or a
/// data/reflection-security.json class's.
fn members_of(class: &Class) -> Members {
    let Some(dump) = class.dump else {
        let Some(security) = class.security else { return Members::default() };
        let file: FileMembers = read(security);
        return Members {
            properties: file.properties.into_iter().map(property).collect(),
            methods: file.methods.into_iter().map(method).collect(),
            events: file.events.into_iter().map(event).collect(),
        };
    };
    let file: FileMembers = read(dump);
    let ext: Option<FileExtend> = class.security.map(read);
    let mut properties: Vec<Property> = Vec::with_capacity(file.properties.len());
    let mut methods: Vec<Method> = file.methods.into_iter().map(method).collect();
    let mut events: Vec<Event> = file.events.into_iter().map(event).collect();
    for file_property in file.properties {
        let visible = ext.as_ref().and_then(|ext| ext.visible.get(&file_property.name)).map(|text| caps_of(text));
        let mut property = property(file_property);
        if !property.permits.iter().any(|(action, _)| action.0 == "Read") {
            property.visible = visible.unwrap_or(0);
        }
        properties.push(property);
    }
    if let Some(ext) = ext {
        // (the hidden properties, each at its place among the class's own when all are shown)
        let mut hidden = ext.properties;
        hidden.sort_by_key(|property| property.at.unwrap_or(usize::MAX));
        for file_property in hidden {
            let at = file_property.at.unwrap_or(properties.len()).min(properties.len());
            properties.insert(at, property(file_property));
        }
        for (name, more) in &ext.permits.properties {
            if let Some(property) = properties.iter_mut().find(|property| &*property.name == name.as_str()) {
                add_permits(&mut property.permits, more);
            }
        }
        for (name, more) in &ext.permits.methods {
            if let Some(method) = methods.iter_mut().find(|method| &*method.name == name.as_str()) {
                add_permits(&mut method.permits, more);
            }
        }
        for (name, more) in &ext.permits.events {
            if let Some(event) = events.iter_mut().find(|event| &*event.name == name.as_str()) {
                add_permits(&mut event.permits, more);
            }
        }
    }
    Members { properties: properties.into_boxed_slice(), methods: methods.into_boxed_slice(), events: events.into_boxed_slice() }
}

struct Built {
    class: Class,
    superclass: Option<String>,
}

pub(crate) fn build(dump: &'static [u8], security: &'static [u8]) -> Data {
    let dump: DumpFile = serde_json::from_slice(dump).expect("data/reflection.json reads");
    let security: SecurityFile = serde_json::from_slice(security).expect("data/reflection-security.json reads");
    let mut extend: FxHashMap<String, (&'static RawValue, Option<String>)> = FxHashMap::default();
    for raw in security.extend {
        let head: FileExtendHead = read(raw);
        extend.insert(head.name, (raw, head.list));
    }
    let mut built: Vec<Built> = Vec::with_capacity(dump.classes.len() + security.classes.len());
    for raw in dump.classes {
        let file: FileClass = read(raw);
        let ext = extend.remove(&file.name);
        let class_permits = permits(&file.permits);
        let list = list_caps(&class_permits, ext.as_ref().and_then(|(_, list)| list.as_deref()));
        built.push(Built {
            class: Class {
                name: file.name.into(),
                superclass: None,
                serialized: file.serialized.unwrap_or(false),
                display: display(&file.display),
                permits: class_permits,
                listed: true,
                answers: true,
                list,
                members: list,
                subclasses: Box::new([]),
                dump: Some(raw),
                security: ext.map(|(raw, _)| raw),
                own: OnceLock::new(),
            },
            superclass: file.superclass,
        });
    }
    for raw in security.classes {
        let file: FileClass = read(raw);
        let class_permits = permits(&file.permits);
        let listed = file.listed.unwrap_or(true) && !file.subclass;
        let list = if listed { list_caps(&class_permits, file.list.as_deref()) } else { 0 };
        let members = file.members.as_deref().map(caps_of).unwrap_or(list);
        built.push(Built {
            class: Class {
                name: file.name.into(),
                superclass: None,
                serialized: file.serialized.unwrap_or(false),
                display: display(&file.display),
                permits: class_permits,
                listed,
                answers: !file.subclass,
                list,
                members,
                subclasses: Box::new([]),
                dump: None,
                security: Some(raw),
                own: OnceLock::new(),
            },
            superclass: file.superclass,
        });
    }
    built.sort_by(|a, b| a.class.name.cmp(&b.class.name));
    let by_name: FxHashMap<Box<str>, u32> = built.iter().enumerate().map(|(index, built)| (built.class.name.clone(), index as u32)).collect();
    let mut children: Vec<Vec<u32>> = vec![Vec::new(); built.len()];
    for (index, entry) in built.iter_mut().enumerate() {
        if let Some(superclass) = entry.superclass.as_deref().and_then(|name| by_name.get(name)) {
            entry.class.superclass = Some(*superclass);
            children[*superclass as usize].push(index as u32);
        }
    }
    let classes: Box<[Class]> = built
        .into_iter()
        .zip(children)
        .map(|(entry, subclasses)| Class { subclasses: subclasses.into_boxed_slice(), ..entry.class })
        .collect();
    Data { classes, by_name }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plugin's set is a game script's with Plugin and PluginOrOpenCloud; a lacking capability is named by the first missing one.
    #[test]
    fn a_plugins_capabilities_and_the_one_a_set_lacks() {
        assert_eq!(PLUGIN_CAPS, SCRIPT_CAPS | caps_of("Plugin | PluginOrOpenCloud"));
        assert_eq!(first_lacking(SCRIPT_CAPS, caps_of("RobloxScript | AvatarAppearance")), Some("RobloxScript"));
        assert_eq!(first_lacking(SCRIPT_CAPS, caps_of("Basic | PluginOrOpenCloud")), Some("PluginOrOpenCloud"));
        assert_eq!(first_lacking(PLUGIN_CAPS, caps_of("Basic | PluginOrOpenCloud")), None);
        assert_eq!(first_lacking(SCRIPT_CAPS, caps_of("RobloxScript | InternalTest")), Some("RobloxScript"));
        assert_eq!(first_lacking(SCRIPT_CAPS, caps_of("")), None);
        assert_eq!(first_lacking(SCRIPT_CAPS, caps_of("Unassigned")), None);
    }

    /// A game script's view of the data (its own set, no Security filter) is data/reflection.json, member for member and permit
    /// for permit: the hidden members data/reflection-security.json adds go in their places without moving another, and the
    /// counts are a 0.747 client's (660 classes listed, 941 with every capability, 671 whose members a game script may query).
    #[test]
    fn a_game_scripts_view_of_the_data_is_the_dump() {
        let data = data();
        let dump: serde_json::Value = serde_json::from_slice(crate::data::REFLECTION).unwrap();
        let shown = |permits: &Permits| -> Vec<(String, Caps)> {
            let mut list: Vec<(String, Caps)> = permits.iter().filter(|(_, caps)| holds(SCRIPT_CAPS, *caps)).map(|(action, caps)| (action.0.to_string(), *caps)).collect();
            list.sort();
            list
        };
        let dumped = |permits: &serde_json::Value| -> Vec<(String, Caps)> {
            let mut list: Vec<(String, Caps)> = permits.as_object().map(|map| map.iter().map(|(action, text)| (action.clone(), caps_of(text.as_str().unwrap()))).collect()).unwrap_or_default();
            list.sort();
            list
        };
        for class in dump["classes"].as_array().unwrap() {
            let name = class["name"].as_str().unwrap();
            let ours = &data.classes[data.index(name).unwrap() as usize];
            assert!(ours.listed && holds(SCRIPT_CAPS, ours.list) && holds(SCRIPT_CAPS, ours.members), "{name}");
            let own = data.members(data.index(name).unwrap());
            let properties: Vec<&Property> = own.properties.iter().filter(|property| holds(SCRIPT_CAPS, property.visible)).collect();
            let names: Vec<&str> = class["properties"].as_array().unwrap().iter().map(|property| property["name"].as_str().unwrap()).collect();
            assert_eq!(properties.iter().map(|property| &*property.name).collect::<Vec<_>>(), names, "{name}");
            for (property, file) in properties.iter().zip(class["properties"].as_array().unwrap()) {
                assert_eq!(shown(&property.permits), dumped(&file["permits"]), "{name}.{}", property.name);
            }
            for (kind, list) in [("methods", own.methods.iter().map(|m| (&*m.name, &m.permits)).collect::<Vec<_>>()), ("events", own.events.iter().map(|e| (&*e.name, &e.permits)).collect())] {
                let file = class[kind].as_array().unwrap();
                assert_eq!(list.iter().map(|(name, _)| *name).collect::<Vec<_>>(), file.iter().map(|m| m["name"].as_str().unwrap()).collect::<Vec<_>>(), "{name}.{kind}");
                for ((member, permits), file) in list.iter().zip(file) {
                    assert_eq!(shown(permits), dumped(&file["permits"]), "{name}.{member}");
                }
            }
        }
        let all = (1u64 << CAPABILITY_ORDER.len()) - 1;
        assert_eq!(data.classes.iter().filter(|class| class.listed && holds(SCRIPT_CAPS, class.list)).count(), 660);
        assert_eq!(data.classes.iter().filter(|class| class.listed && holds(all, class.list)).count(), 941);
        assert_eq!(data.classes.iter().filter(|class| class.answers && holds(SCRIPT_CAPS, class.members)).count(), 671);
    }
}
