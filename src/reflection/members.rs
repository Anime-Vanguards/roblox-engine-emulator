//! What the reflection knows about members beyond what the engine runs on: methods, events and callbacks with their
//! parameters, types, permits and display data, rbx-dom's tags and serialization, and the lookups an editor needs
//! (re-exported from [`crate::reflection`], whose module documentation has an example).
//!
//! The engine itself reads none of this. A method's, event's or callback's info is built on the first call that asks for
//! one (the compiled-in dump is read again for it, some milliseconds), so an engine that never asks boots as it did.

use std::{borrow::Cow, fmt, ptr};

use rustc_hash::FxHashMap;
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use super::{ClassId, ClassInfo, EnumId, EnumItemId, EnumInfo, EnumItemInfo, ITEM_ALIASES, NameId, PropertyInfo, Reflection, list, text};
use crate::value::Value;

/// Where the reflection's knowledge of a member comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MemberOrigin {
    /// Studio's ReflectionService, as `data/reflection.json` records it: every field this module describes.
    Dump,
    /// rbx-dom's database, for a class the dump does not list (Player, UserGameSettings): a property's type and
    /// [`DomProperty`] data, but no display data or permits.
    RbxDom,
    /// The engine's own lists: a member the dump leaves out that the engine has (measured, or documented by Roblox). Its
    /// name and class are known, its parameters and types are not (none are listed), and it has no permits or display data.
    Engine,
}

/// The kind of a member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MemberKind {
    Property,
    Method,
    Event,
    Callback,
}

/// Display data as the dump records it (ReflectionService's `Display`): (key, text) pairs. Studio fills two keys:
/// `Category`, the group the Properties panel files a property under (every class and property of the dump has one), and
/// `DeprecationMessage`, present on a deprecated class or member (its text may be empty).
pub type DisplayData = Box<[(Box<str>, Box<str>)]>;

/// The text of `key` in display data.
pub fn display_text<'a>(display: &'a [(Box<str>, Box<str>)], key: &str) -> Option<&'a str> {
    display.iter().find(|(name, _)| &**name == key).map(|(_, text)| &**text)
}

/// Display data from the dump's `display` table (`[]` when empty).
pub(super) fn display_from_json(value: Option<&serde_json::Value>) -> DisplayData {
    match value {
        Some(serde_json::Value::Object(display)) => {
            display.iter().filter_map(|(key, value)| Some((key.as_str().into(), value.as_str()?.into()))).collect()
        }
        _ => Box::new([]),
    }
}

macro_rules! display_accessors {
    ($($owner:ty),* $(,)?) => {$(
        impl $owner {
            /// The group Studio's Properties panel and the API reference file it under (`Display.Category`: "Appearance",
            /// "Data", "Behavior"); `None` where the dump records none (members other than properties, classes only
            /// rbx-dom describes).
            pub fn category(&self) -> Option<&str> {
                display_text(&self.display, "Category")
            }

            /// Why it is deprecated and what to use instead (`Display.DeprecationMessage`: "Use `GetChildren` instead."), when
            /// it is deprecated. The text may be empty: a deprecated item need not say why.
            pub fn deprecation_message(&self) -> Option<&str> {
                display_text(&self.display, "DeprecationMessage")
            }

            /// Whether the dump marks it deprecated (it has a [`Self::deprecation_message`], empty or not).
            pub fn is_deprecated(&self) -> bool {
                self.deprecation_message().is_some()
            }
        }
    )*};
}

display_accessors!(ClassInfo, PropertyInfo, MethodInfo, EventInfo, CallbackInfo);

/// The actions the dump records permits for: their names are kept without an allocation of their own.
const ACTIONS: &[&str] = &["New", "GetService", "Read", "Write", "ReadParallel", "WriteParallel", "Call", "CallParallel", "Listen", "ListenParallel"];

/// What a game script may do with a class or member, and the security capabilities each action needs: ReflectionService's
/// `Permits` as the dump records them. The actions are `New` and `GetService` for a class, `Read`, `Write` and
/// `ReadParallel` for a property, `Call` and `CallParallel` for a method and `Listen` for an event. An action a game script
/// may not take is absent: a member only plugins or Roblox's own scripts use permits nothing (Selection:Get), and a
/// read-only property has no `Write`. A method has `CallParallel` when a parallel (desynchronized) thread may call it, as
/// Roblox's API reference marks it thread safe (GetChildren and Raycast have it, WaitForChild and Destroy do not); only 38
/// controller properties have `ReadParallel`, so its absence says nothing about a property (BasePart.Position, read safe in
/// the API reference, has none). Each action's capabilities are the text `tostring` gives its SecurityCapabilities: empty
/// when it needs none, else their names joined by ` | ` (`"Basic | UI"`), which a sandboxed script must have.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Permits(Box<[(Cow<'static, str>, Box<str>)]>);

impl Permits {
    /// The dump's `permits` table (`[]` when empty), in action order.
    pub(super) fn from_json(value: Option<&serde_json::Value>) -> Permits {
        let Some(serde_json::Value::Object(map)) = value else { return Permits::default() };
        let mut permits: Vec<(Cow<'static, str>, Box<str>)> = map
            .iter()
            .filter_map(|(action, capabilities)| {
                let action = match ACTIONS.iter().find(|known| **known == action.as_str()) {
                    Some(known) => Cow::Borrowed(*known),
                    None => Cow::Owned(action.clone()),
                };
                Some((action, capabilities.as_str()?.into()))
            })
            .collect();
        permits.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        Permits(permits.into_boxed_slice())
    }

    /// The capabilities text of `action` ("" when it needs none), or `None` when a game script may not take it.
    pub fn get(&self, action: &str) -> Option<&str> {
        self.0.iter().find(|(name, _)| name == action).map(|(_, capabilities)| &**capabilities)
    }

    /// Whether a game script may take `action` (with the capabilities it needs, when sandboxed).
    pub fn allows(&self, action: &str) -> bool {
        self.get(action).is_some()
    }

    /// The names of the capabilities `action` needs (`"Basic | UI"`: Basic, UI); none when it needs none or is not
    /// permitted at all (tell the two apart with [`Self::allows`]).
    pub fn capabilities(&self, action: &str) -> impl Iterator<Item = &str> {
        self.get(action).unwrap_or("").split('|').map(str::trim).filter(|name| !name.is_empty())
    }

    /// Every (action, capabilities text) pair, in action name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(action, capabilities)| (&**action, &**capabilities))
    }

    /// No action is permitted.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A member's type as ReflectionService gives it (ReflectedType).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TypeInfo {
    /// The type a script sees (`ScriptType`): "number", "string", "Instance", "Instances", "EnumItem", "Tuple",
    /// "RaycastResult?" (`?`: it may be nil). Empty where the dump names none: a method that returns nothing (engine type
    /// "Void") and some engine types scripts see as themselves ("Variant", "User").
    pub script: Box<str>,
    /// The engine's own type (`EngineType`): "float", "int64", "bool", "RefType", "Instance", "Void", "Variant".
    pub engine: Box<str>,
    /// The enum an EnumItem type is of (`EnumType`).
    pub enum_name: Option<Box<str>>,
    /// The class an Instance or Instances type names (`InstanceType`): Model.PrimaryPart's is BasePart, the argument of
    /// BasePart.Touched BasePart, the result of GetChildren Instance.
    pub instance_type: Option<Box<str>>,
}

impl TypeInfo {
    fn from_json(value: Option<&serde_json::Value>) -> TypeInfo {
        let field = |key: &str| text(value.and_then(|value| value.get(key)));
        TypeInfo {
            script: field("script").unwrap_or("").into(),
            engine: field("engine").unwrap_or("").into(),
            enum_name: field("enum").map(Into::into),
            instance_type: field("instance").map(Into::into),
        }
    }
}

/// A parameter of a method, event or callback.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ParameterInfo {
    /// Its name as Roblox's API names it (`recursive`, `timeOut`)
    pub name: Box<str>,
    pub type_info: TypeInfo,
    /// Its `DefaultValue`, encoded as the dump encodes defaults (docs/engine.md "Data files"): a JSON number, string or
    /// boolean, or an object whose `$t` names the type (`{"$t": "EnumItem", "e": "OutfitType", "n": "All"}`, `{"$t":
    /// "Vector3", "v": [0, 0, 0]}`; a table's items under `v`, a datatype ree has no encoding for as `tostring` writes it
    /// under `s`). `None` where ReflectionService gives none, which does not make the parameter required: the dump does not
    /// say which are optional (WaitForChild's timeOut has no default). [`Reflection::parameter_default`] decodes it.
    pub default: Option<serde_json::Value>,
}

impl ParameterInfo {
    fn from_json(value: &serde_json::Value) -> ParameterInfo {
        ParameterInfo {
            name: text(value.get("name")).unwrap_or("").into(),
            type_info: TypeInfo::from_json(value.get("type")),
            default: value.get("default").cloned(),
        }
    }

    /// The default as Luau would write it, for a signature (`false`, `1`, `""`, `Enum.OutfitType.All`,
    /// `Vector3.new(0, 0, 0)`, `{}`); `None` where there is none.
    pub fn default_text(&self) -> Option<String> {
        self.default.as_ref().map(luau_text)
    }
}

fn luau_text(value: &serde_json::Value) -> String {
    use serde_json::Value as J;
    match value {
        J::String(text) => format!("{text:?}"),
        J::Object(object) => match (object.get("$t").and_then(J::as_str), object.get("v"), object.get("s")) {
            (Some("EnumItem"), _, _) => format!("Enum.{}.{}", text(object.get("e")).unwrap_or("?"), text(object.get("n")).unwrap_or("?")),
            (Some("number"), _, Some(J::String(special))) => match special.as_str() {
                "inf" => "math.huge".into(),
                "-inf" => "-math.huge".into(),
                _ => "0/0".into(),
            },
            (Some("table"), Some(J::Object(items)), _) => {
                let mut parts: Vec<String> = items.iter().map(|(key, item)| format!("{key} = {}", luau_text(item))).collect();
                parts.sort();
                format!("{{{}}}", parts.join(", "))
            }
            (Some("table"), _, _) => "{}".into(),
            (Some(kind), Some(J::Array(parts)), _) => {
                let parts: Vec<String> = parts.iter().map(luau_text).collect();
                format!("{kind}.new({})", parts.join(", "))
            }
            (Some(kind), Some(part), _) => format!("{kind}.new({})", luau_text(part)),
            (_, _, Some(J::String(written))) => written.clone(),
            _ => value.to_string(),
        },
        other => other.to_string(),
    }
}

/// A method of a class (ReflectedMethod).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct MethodInfo {
    pub name: NameId,
    /// The class that declares it
    pub owner: ClassId,
    /// Its parameters in order (none for a [`MemberOrigin::Engine`] method, whose parameters are not recorded)
    pub parameters: Box<[ParameterInfo]>,
    /// What it returns (`ReturnType`); `None` for a [`MemberOrigin::Engine`] method
    pub returns: Option<TypeInfo>,
    /// `Call` and `CallParallel`, with their capabilities ([`Permits`])
    pub permits: Permits,
    /// Its display data ([`DisplayData`]: a DeprecationMessage when deprecated)
    pub display: DisplayData,
    pub origin: MemberOrigin,
}

impl MethodInfo {
    /// A game script may call it from a parallel (desynchronized) thread: the dump permits `CallParallel`, as it does the
    /// methods Roblox's API reference marks thread safe.
    pub fn callable_in_parallel(&self) -> bool {
        self.permits.allows("CallParallel")
    }
}

/// An event of a class (ReflectedEvent): the RBXScriptSignal `instance.<Name>` reads.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct EventInfo {
    pub name: NameId,
    /// The class that declares it
    pub owner: ClassId,
    /// The values it fires with, in order
    pub parameters: Box<[ParameterInfo]>,
    /// `Listen`, with its capabilities ([`Permits`])
    pub permits: Permits,
    /// Its display data ([`DisplayData`]: a DeprecationMessage when deprecated)
    pub display: DisplayData,
    pub origin: MemberOrigin,
}

/// A callback of a class: a function a script assigns for the engine to call (`BindableFunction.OnInvoke`).
/// ReflectionService lists no callbacks (Studio 0.747 has no GetCallbacksOfClass), so every callback is a
/// [`MemberOrigin::Engine`] one, whose parameters and return type are not recorded.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CallbackInfo {
    pub name: NameId,
    /// The class that declares it
    pub owner: ClassId,
    pub parameters: Box<[ParameterInfo]>,
    pub returns: Option<TypeInfo>,
    pub permits: Permits,
    pub display: DisplayData,
    pub origin: MemberOrigin,
}

/// A member of a class, of any kind.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ReflectedMember<'a> {
    Property(&'a PropertyInfo),
    Method(&'a MethodInfo),
    Event(&'a EventInfo),
    Callback(&'a CallbackInfo),
}

impl PartialEq for ReflectedMember<'_> {
    /// The same member (the same entry of the reflection, not an equal one).
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Property(a), Self::Property(b)) => ptr::eq(*a, *b),
            (Self::Method(a), Self::Method(b)) => ptr::eq(*a, *b),
            (Self::Event(a), Self::Event(b)) => ptr::eq(*a, *b),
            (Self::Callback(a), Self::Callback(b)) => ptr::eq(*a, *b),
            _ => false,
        }
    }
}

impl<'a> ReflectedMember<'a> {
    pub fn kind(&self) -> MemberKind {
        match self {
            Self::Property(_) => MemberKind::Property,
            Self::Method(_) => MemberKind::Method,
            Self::Event(_) => MemberKind::Event,
            Self::Callback(_) => MemberKind::Callback,
        }
    }

    pub fn name(&self) -> NameId {
        match self {
            Self::Property(info) => info.name,
            Self::Method(info) => info.name,
            Self::Event(info) => info.name,
            Self::Callback(info) => info.name,
        }
    }

    /// The class that declares it.
    pub fn owner(&self) -> ClassId {
        match self {
            Self::Property(info) => info.owner,
            Self::Method(info) => info.owner,
            Self::Event(info) => info.owner,
            Self::Callback(info) => info.owner,
        }
    }

    pub fn display(&self) -> &'a [(Box<str>, Box<str>)] {
        match self {
            Self::Property(info) => &info.display,
            Self::Method(info) => &info.display,
            Self::Event(info) => &info.display,
            Self::Callback(info) => &info.display,
        }
    }

    /// `Display.Category` (properties have one; [`DisplayData`]).
    pub fn category(&self) -> Option<&'a str> {
        display_text(self.display(), "Category")
    }

    /// `Display.DeprecationMessage`, when it is deprecated ([`DisplayData`]).
    pub fn deprecation_message(&self) -> Option<&'a str> {
        display_text(self.display(), "DeprecationMessage")
    }

    pub fn is_deprecated(&self) -> bool {
        self.deprecation_message().is_some()
    }

    pub fn permits(&self) -> &'a Permits {
        match self {
            Self::Property(info) => &info.permits,
            Self::Method(info) => &info.permits,
            Self::Event(info) => &info.permits,
            Self::Callback(info) => &info.permits,
        }
    }

    pub fn origin(&self) -> MemberOrigin {
        match self {
            Self::Property(info) => info.origin,
            Self::Method(info) => info.origin,
            Self::Event(info) => info.origin,
            Self::Callback(info) => info.origin,
        }
    }

    /// Its parameters (none for a property).
    pub fn parameters(&self) -> &'a [ParameterInfo] {
        match self {
            Self::Property(_) => &[],
            Self::Method(info) => &info.parameters,
            Self::Event(info) => &info.parameters,
            Self::Callback(info) => &info.parameters,
        }
    }
}

/// A class's tags in rbx-dom's database (rbx_reflection_database, which takes them from an older Roblox API dump than
/// `data/reflection.json`'s: [`Reflection::dom_version`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ClassTags {
    pub deprecated: bool,
    /// Studio's Explorer and object browser hide it
    pub not_browsable: bool,
    pub not_creatable: bool,
    /// The server does not replicate it to clients
    pub not_replicated: bool,
    /// Replicated to its player only
    pub player_replicated: bool,
    pub service: bool,
    /// A `settings()` class
    pub settings: bool,
    /// A `UserSettings()` class
    pub user_settings: bool,
}

/// A property's tags in rbx-dom's database ([`ClassTags`] says which).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PropertyTags {
    pub deprecated: bool,
    /// Studio's Properties panel does not show it
    pub hidden: bool,
    /// The object browser does not list it
    pub not_browsable: bool,
    /// The server does not replicate it to clients
    pub not_replicated: bool,
    pub not_scriptable: bool,
    pub read_only: bool,
    pub write_only: bool,
}

/// What scripts may do with a property, by rbx-dom's database.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Scriptability {
    None,
    Read,
    Write,
    ReadWrite,
    /// Scripts reach it, but through a path of its own (rbx-dom handles it specially: Model.Scale, Terrain.MaterialColors)
    Custom,
}

/// How a place or model file stores a property, by rbx-dom's database.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Serialization {
    /// Under its own name
    Serializes,
    /// Not at all
    DoesNotSerialize,
    /// Under another name (BasePart.Size as `size`)
    SerializesAs(&'static str),
    /// Files hold it as another property it is migrated to on load (TextLabel.Font as FontFace)
    MigratesTo(&'static str),
    /// It is another name of the property named here (BasePart.Color is Color3uint8 in files): files hold that one
    AliasFor(&'static str),
}

/// rbx-dom's description of a property: what its database (an older Roblox API dump, [`Reflection::dom_version`]) adds
/// to the dump's, for reading and writing place and model files.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DomProperty {
    pub name: &'static str,
    /// The class rbx-dom files it under
    pub class: &'static str,
    /// Its datatype as rbx-dom stores it (`Vector3`, `Float32`, `Enum.Material`, `BinaryString`)
    pub data_type: Option<Box<str>>,
    pub tags: PropertyTags,
    pub scriptability: Scriptability,
    pub serialization: Serialization,
}

fn dom_property(class: &'static str, name: &'static str, descriptor: &'static rbx_reflection::PropertyDescriptor<'static>) -> DomProperty {
    use rbx_reflection::{PropertyKind, PropertySerialization, PropertyTag as T};
    let has = |tag| descriptor.tags.contains(&tag);
    DomProperty {
        name,
        class,
        data_type: super::lune_datatype_name(&descriptor.data_type).map(Into::into),
        tags: PropertyTags {
            deprecated: has(T::Deprecated),
            hidden: has(T::Hidden),
            not_browsable: has(T::NotBrowsable),
            not_replicated: has(T::NotReplicated),
            not_scriptable: has(T::NotScriptable),
            read_only: has(T::ReadOnly),
            write_only: has(T::WriteOnly),
        },
        scriptability: match descriptor.scriptability {
            rbx_reflection::Scriptability::None => Scriptability::None,
            rbx_reflection::Scriptability::Read => Scriptability::Read,
            rbx_reflection::Scriptability::Write => Scriptability::Write,
            rbx_reflection::Scriptability::ReadWrite => Scriptability::ReadWrite,
            _ => Scriptability::Custom,
        },
        serialization: match &descriptor.kind {
            PropertyKind::Alias { alias_for } => Serialization::AliasFor(alias_for),
            PropertyKind::Canonical { serialization } => match serialization {
                PropertySerialization::Serializes => Serialization::Serializes,
                PropertySerialization::SerializesAs(other) => Serialization::SerializesAs(other),
                PropertySerialization::Migrate(migration) => Serialization::MigratesTo(&migration.new_property_name),
                _ => Serialization::DoesNotSerialize,
            },
            _ => Serialization::DoesNotSerialize,
        },
    }
}

/// The methods, events and callbacks of every class, built on first use ([`Reflection::members`]).
#[derive(Debug, Default)]
pub(super) struct Members {
    methods: Vec<MethodInfo>,
    events: Vec<EventInfo>,
    callbacks: Vec<CallbackInfo>,
    /// Per class: where its own methods, events and callbacks are in the lists above (start, end), in its order
    own: Vec<[(u32, u32); 3]>,
    /// (declaring class, name) to the index in each list
    declared: [FxHashMap<(ClassId, NameId), u32>; 3],
}

const METHODS: usize = 0;
const EVENTS: usize = 1;
const CALLBACKS: usize = 2;

/// A class entry of the dump, as the members are built from it: its name and member lists (other fields skipped).
#[derive(serde::Deserialize)]
struct MemberClass {
    #[serde(default)]
    name: Option<serde_json::Value>,
    #[serde(default)]
    events: Option<serde_json::Value>,
    #[serde(default)]
    methods: Option<serde_json::Value>,
    #[serde(default)]
    callbacks: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct MemberDump {
    #[serde(default)]
    classes: Vec<MemberClass>,
}

/// A class's own entries of one kind in the dump, by name (the first of a name).
fn by_name(list: Option<&serde_json::Value>) -> FxHashMap<&str, &serde_json::Value> {
    let mut found = FxHashMap::default();
    for member in super::list(list) {
        if let Some(name) = text(member.get("name")) {
            found.entry(name).or_insert(member);
        }
    }
    found
}

impl Members {
    /// The members of every class of `reflection`, from the dump it was read from (its own lists give each class's
    /// members and order; the dump's entry of the same name the rest; an engine member has none).
    pub(super) fn build(reflection: &Reflection, dump: &[u8]) -> Members {
        let classes = serde_json::from_slice::<MemberDump>(dump).map(|dump| dump.classes).unwrap_or_default();
        let mut entries: FxHashMap<&str, &MemberClass> = FxHashMap::default();
        for class in &classes {
            if let Some(name) = text(class.name.as_ref()) {
                entries.insert(name, class);
            }
        }
        let mut members = Members::default();
        for class in &reflection.classes {
            let entry = if class.reflected { entries.get(&*class.name).copied() } else { None };
            let parameters = |member: Option<&serde_json::Value>| -> Box<[ParameterInfo]> {
                list(member.and_then(|member| member.get("parameters"))).iter().map(ParameterInfo::from_json).collect()
            };
            let permits = |member: Option<&serde_json::Value>| Permits::from_json(member.and_then(|member| member.get("permits")));
            let display = |member: Option<&serde_json::Value>| display_from_json(member.and_then(|member| member.get("display")));
            let origin = |member: Option<&serde_json::Value>| if member.is_some() { MemberOrigin::Dump } else { MemberOrigin::Engine };
            let mut own = [(0u32, 0u32); 3];

            let dumped = by_name(entry.and_then(|entry| entry.methods.as_ref()));
            own[METHODS].0 = members.methods.len() as u32;
            for &name in &class.own_methods {
                let member = dumped.get(reflection.name(name)).copied();
                members.declared[METHODS].entry((class.id, name)).or_insert(members.methods.len() as u32);
                members.methods.push(MethodInfo {
                    name,
                    owner: class.id,
                    parameters: parameters(member),
                    returns: member.map(|member| TypeInfo::from_json(member.get("returns"))),
                    permits: permits(member),
                    display: display(member),
                    origin: origin(member),
                });
            }
            own[METHODS].1 = members.methods.len() as u32;

            let dumped = by_name(entry.and_then(|entry| entry.events.as_ref()));
            own[EVENTS].0 = members.events.len() as u32;
            for &name in &class.own_events {
                let member = dumped.get(reflection.name(name)).copied();
                members.declared[EVENTS].entry((class.id, name)).or_insert(members.events.len() as u32);
                members.events.push(EventInfo {
                    name,
                    owner: class.id,
                    parameters: parameters(member),
                    permits: permits(member),
                    display: display(member),
                    origin: origin(member),
                });
            }
            own[EVENTS].1 = members.events.len() as u32;

            let dumped = by_name(entry.and_then(|entry| entry.callbacks.as_ref()));
            own[CALLBACKS].0 = members.callbacks.len() as u32;
            for &name in &class.own_callbacks {
                let member = dumped.get(reflection.name(name)).copied();
                members.declared[CALLBACKS].entry((class.id, name)).or_insert(members.callbacks.len() as u32);
                members.callbacks.push(CallbackInfo {
                    name,
                    owner: class.id,
                    parameters: parameters(member),
                    returns: member.map(|member| TypeInfo::from_json(member.get("returns"))),
                    permits: permits(member),
                    display: display(member),
                    origin: origin(member),
                });
            }
            own[CALLBACKS].1 = members.callbacks.len() as u32;
            members.own.push(own);
        }
        members
    }
}

impl Reflection {
    /// The methods, events and callbacks, built on the first call.
    fn member_store(&self) -> &Members {
        self.members.get_or_init(|| self.member_source.map(|dump| Members::build(self, dump)).unwrap_or_default())
    }

    /// Every class, in id order: the dump's in the dump's order (ReflectionService's, by name), then those only rbx-dom
    /// describes, by name. [`ClassInfo::reflected`] tells them apart.
    pub fn all_classes(&self) -> impl Iterator<Item = &ClassInfo> {
        self.classes.iter()
    }

    /// Every enum, in the dump's order (`Enum:GetEnums()`'s).
    pub fn all_enums(&self) -> impl Iterator<Item = &EnumInfo> {
        self.enums.iter()
    }

    pub fn enum_by_name(&self, name: &str) -> Option<&EnumInfo> {
        self.enum_id(name).map(|id| &self.enums[id as usize])
    }

    /// The items of an enum, in `GetEnumItems` order (legacy names not included: [`Self::legacy_names`]).
    pub fn enum_items_of(&self, id: EnumId) -> impl Iterator<Item = &EnumItemInfo> {
        self.enums[id as usize].items.iter().map(|&item| &self.enum_items[item as usize])
    }

    /// The legacy names that read as `item` (`Enum.Font.Montserrat` is Gotham, `Enum.KeyCode.Unknown` None): Roblox's API
    /// dump's LegacyNames, each measured in a 0.747 client. No enumeration, FromName or FromValue answers one.
    pub fn legacy_names(&self, item: EnumItemId) -> impl Iterator<Item = &'static str> + '_ {
        let info = &self.enum_items[item as usize];
        let enum_name = &self.enums[info.enum_type as usize].name;
        ITEM_ALIASES
            .iter()
            .filter(move |(name, _)| *name == &**enum_name)
            .flat_map(|(_, aliases)| aliases.iter())
            .filter(move |(_, target)| *target == &*info.name)
            .map(|(alias, _)| *alias)
    }

    /// The classes `class` derives from and itself, the root first (Object, Instance, ..., `class`).
    pub fn class_chain(&self, class: ClassId) -> Vec<ClassId> {
        let mut chain = Vec::new();
        let mut current = Some(class);
        while let Some(id) = current {
            chain.push(id);
            current = self.classes[id as usize].superclass;
        }
        chain.reverse();
        chain
    }

    /// The properties `class` declares, in the dump's order (then the ones the engine adds).
    pub fn own_properties(&self, class: ClassId) -> impl Iterator<Item = &PropertyInfo> {
        self.classes[class as usize].own_properties.iter().map(|&index| &self.properties[index as usize])
    }

    /// The methods `class` declares, in the dump's order (then the ones the engine adds).
    pub fn own_methods(&self, class: ClassId) -> &[MethodInfo] {
        let (start, end) = self.member_store().own[class as usize][METHODS];
        &self.member_store().methods[start as usize..end as usize]
    }

    /// The events `class` declares, in the dump's order (then the ones the engine adds).
    pub fn own_events(&self, class: ClassId) -> &[EventInfo] {
        let (start, end) = self.member_store().own[class as usize][EVENTS];
        &self.member_store().events[start as usize..end as usize]
    }

    /// The callbacks `class` declares.
    pub fn own_callbacks(&self, class: ClassId) -> &[CallbackInfo] {
        let (start, end) = self.member_store().own[class as usize][CALLBACKS];
        &self.member_store().callbacks[start as usize..end as usize]
    }

    /// The members `class` declares: its properties, then its methods, events and callbacks, each kind in the dump's order.
    pub fn own_members(&self, class: ClassId) -> impl Iterator<Item = ReflectedMember<'_>> {
        self.own_properties(class)
            .map(ReflectedMember::Property)
            .chain(self.own_methods(class).iter().map(ReflectedMember::Method))
            .chain(self.own_events(class).iter().map(ReflectedMember::Event))
            .chain(self.own_callbacks(class).iter().map(ReflectedMember::Callback))
    }

    /// Every member of `class`, its own and inherited, in a stable order: class by class from the root of its chain to
    /// `class` itself ([`Self::class_chain`]: Instance's first), each class's as [`Self::own_members`] lists them. A member
    /// a class declares again (a value object's Changed, CollectionService's AddTag) is listed once, where that class lists
    /// it, as [`Self::member`] finds it.
    pub fn members(&self, class: ClassId) -> impl Iterator<Item = ReflectedMember<'_>> {
        self.class_chain(class)
            .into_iter()
            .flat_map(move |owner| self.own_members(owner))
            .filter(move |member| self.member_of_kind(class, member.kind(), member.name()) == Some(*member))
    }

    fn declared<'a, T>(&'a self, kind: usize, list: &'a [T], class: ClassId, name: NameId) -> Option<&'a T> {
        let declared = &self.member_store().declared[kind];
        let mut current = Some(class);
        while let Some(id) = current {
            if let Some(&index) = declared.get(&(id, name)) {
                return list.get(index as usize);
            }
            current = self.classes[id as usize].superclass;
        }
        None
    }

    /// The method `name` of `class`, its own or inherited (the nearest class's that declares it).
    pub fn class_method(&self, class: ClassId, name: NameId) -> Option<&MethodInfo> {
        if !self.classes[class as usize].methods.contains(&name) {
            return None;
        }
        self.declared(METHODS, &self.member_store().methods, class, name)
    }

    /// The event `name` of `class`, its own or inherited (the nearest class's that declares it).
    pub fn class_event(&self, class: ClassId, name: NameId) -> Option<&EventInfo> {
        if !self.classes[class as usize].events.contains(&name) {
            return None;
        }
        self.declared(EVENTS, &self.member_store().events, class, name)
    }

    /// The callback `name` of `class`, its own or inherited.
    pub fn class_callback(&self, class: ClassId, name: NameId) -> Option<&CallbackInfo> {
        if !self.classes[class as usize].callbacks.contains(&name) {
            return None;
        }
        self.declared(CALLBACKS, &self.member_store().callbacks, class, name)
    }

    fn member_of_kind(&self, class: ClassId, kind: MemberKind, name: NameId) -> Option<ReflectedMember<'_>> {
        match kind {
            MemberKind::Property => self.class_property(class, name).map(ReflectedMember::Property),
            MemberKind::Method => self.class_method(class, name).map(ReflectedMember::Method),
            MemberKind::Event => self.class_event(class, name).map(ReflectedMember::Event),
            MemberKind::Callback => self.class_callback(class, name).map(ReflectedMember::Callback),
        }
    }

    /// The member `name` of `class`, its own or inherited: a property, else a method, an event or a callback, in the order
    /// an instance's member read looks for them (scriptable or not: [`PropertyInfo::scriptable`] says).
    pub fn member(&self, class: ClassId, name: NameId) -> Option<ReflectedMember<'_>> {
        [MemberKind::Property, MemberKind::Method, MemberKind::Event, MemberKind::Callback]
            .into_iter()
            .find_map(|kind| self.member_of_kind(class, kind, name))
    }

    /// [`Self::member`] by names: `member_by_name("Part", "Anchored")`.
    pub fn member_by_name(&self, class: &str, name: &str) -> Option<ReflectedMember<'_>> {
        self.member(self.class_id(class)?, self.name_id(name)?)
    }

    /// The value a new instance of `class` reads for property `name` (Studio's default in the dump, rbx-dom's for a class
    /// the dump does not list, else the zero value of the property's type), as the engine gives it.
    pub fn default_value(&self, class: ClassId, name: NameId) -> Option<Value> {
        crate::dom::compute_default(self, class, name)
    }

    /// A parameter's default as an engine value, where the engine has its datatype (`None` for a table, a RaycastParams
    /// or OverlapParams, or no default).
    pub fn parameter_default(&self, parameter: &ParameterInfo) -> Option<Value> {
        let encoded = parameter.default.as_ref()?;
        crate::value::decode_default(self, &super::RawDefault::Json(encoded.clone()))
    }

    /// The version of rbx-dom's database ([`ClassTags`], [`DomProperty`]): the Roblox release its API dump came from.
    pub fn dom_version(&self) -> Option<[u32; 4]> {
        rbx_reflection_database::get().ok().map(|database| database.version)
    }

    /// rbx-dom's tags of `class`, where its database has the class.
    pub fn dom_class_tags(&self, class: ClassId) -> Option<ClassTags> {
        use rbx_reflection::ClassTag as T;
        let descriptor = rbx_reflection_database::get().ok()?.classes.get(&*self.classes[class as usize].name)?;
        let has = |tag| descriptor.tags.contains(&tag);
        Some(ClassTags {
            deprecated: has(T::Deprecated),
            not_browsable: has(T::NotBrowsable),
            not_creatable: has(T::NotCreatable),
            not_replicated: has(T::NotReplicated),
            player_replicated: has(T::PlayerReplicated),
            service: has(T::Service),
            settings: has(T::Settings),
            user_settings: has(T::UserSettings),
        })
    }

    /// rbx-dom's description of a property, where its database has it: the property's class's entry, else the nearest of its
    /// superclasses' that has the name (rbx-dom's older dump may file it higher up).
    pub fn dom_property(&self, property: &PropertyInfo) -> Option<DomProperty> {
        let database = rbx_reflection_database::get().ok()?;
        let name = self.name(property.name);
        let mut class = database.classes.get(&*self.classes[property.owner as usize].name);
        while let Some(descriptor) = class {
            if let Some((key, found)) = descriptor.properties.get_key_value(name) {
                return Some(dom_property(&descriptor.name, key, found));
            }
            class = descriptor.superclass.as_ref().and_then(|superclass| database.classes.get(superclass));
        }
        None
    }

    /// Every property rbx-dom's database lists for `class` itself (not its superclasses), by name: the ones files hold that
    /// no script reaches as well (BasePart's `size`, `Color3uint8`; Instance's `Tags`, `AttributesSerialize`).
    pub fn dom_properties(&self, class: ClassId) -> Vec<DomProperty> {
        let Some(descriptor) = rbx_reflection_database::get().ok().and_then(|database| database.classes.get(&*self.classes[class as usize].name)) else {
            return Vec::new();
        };
        let mut properties: Vec<DomProperty> =
            descriptor.properties.iter().map(|(name, property)| dom_property(&descriptor.name, name, property)).collect();
        properties.sort_unstable_by(|a, b| a.name.cmp(b.name));
        properties
    }
}

// The dump as the engine reads it at load. The member lists keep only their names: building JSON values for the methods'
// and events' parameters, types, permits and display only to drop them cost time and memory at every boot. Any shape of
// JSON reads as the JSON values did: a value of another type than expected reads as empty, a repeated key's last value
// counts, an entry without a string name is skipped.

/// A class entry of the dump: the fields the load reads, as JSON values, and the member lists' names.
#[derive(Default)]
pub(super) struct DumpClass {
    pub name: Option<serde_json::Value>,
    pub superclass: Option<serde_json::Value>,
    pub serialized: Option<serde_json::Value>,
    pub permits: Option<serde_json::Value>,
    pub display: Option<serde_json::Value>,
    pub properties: Option<serde_json::Value>,
    pub defaults: Option<serde_json::Value>,
    /// The `name` of each entry of `events`, `methods` and `callbacks` that has a string one, in order
    pub events: Vec<String>,
    pub methods: Vec<String>,
    pub callbacks: Vec<String>,
}

/// The dump's top level, read the same way.
#[derive(Default)]
pub(super) struct Dump {
    pub version: Option<serde_json::Value>,
    pub enums: Option<serde_json::Value>,
    /// The entries of `classes` (an entry that is no object reads as one without a name)
    pub classes: Vec<DumpClass>,
}

/// A key of an object the load reads, matched without an allocation.
enum Key {
    Name,
    Superclass,
    Serialized,
    Permits,
    Display,
    Properties,
    Defaults,
    Events,
    Methods,
    Callbacks,
    Version,
    Enums,
    Classes,
    Other,
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Key, D::Error> {
        struct KeyVisitor;
        impl Visitor<'_> for KeyVisitor {
            type Value = Key;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a key")
            }
            fn visit_str<E: de::Error>(self, key: &str) -> Result<Key, E> {
                Ok(match key {
                    "name" => Key::Name,
                    "superclass" => Key::Superclass,
                    "serialized" => Key::Serialized,
                    "permits" => Key::Permits,
                    "display" => Key::Display,
                    "properties" => Key::Properties,
                    "defaults" => Key::Defaults,
                    "events" => Key::Events,
                    "methods" => Key::Methods,
                    "callbacks" => Key::Callbacks,
                    "version" => Key::Version,
                    "enums" => Key::Enums,
                    "classes" => Key::Classes,
                    _ => Key::Other,
                })
            }
        }
        deserializer.deserialize_identifier(KeyVisitor)
    }
}

/// Implements the visits of every scalar for a reader that takes arrays or objects: each reads as `Default`.
macro_rules! scalars_read_empty {
    ($value:ty) => {
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<$value, E> {
            Ok(Default::default())
        }
        fn visit_i64<E: de::Error>(self, _: i64) -> Result<$value, E> {
            Ok(Default::default())
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> Result<$value, E> {
            Ok(Default::default())
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<$value, E> {
            Ok(Default::default())
        }
        fn visit_str<E: de::Error>(self, _: &str) -> Result<$value, E> {
            Ok(Default::default())
        }
        fn visit_unit<E: de::Error>(self) -> Result<$value, E> {
            Ok(Default::default())
        }
        fn visit_none<E: de::Error>(self) -> Result<$value, E> {
            Ok(Default::default())
        }
    };
}

fn skip_seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<(), A::Error> {
    while seq.next_element::<IgnoredAny>()?.is_some() {}
    Ok(())
}

fn skip_map<'de, A: MapAccess<'de>>(mut map: A) -> Result<(), A::Error> {
    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
    Ok(())
}

/// A member list entry's `name`: the string value of an object's last `name` key, or `None` (an entry that is no object,
/// or whose name is no string).
struct EntryName(Option<String>);

impl<'de> Deserialize<'de> for EntryName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<EntryName, D::Error> {
        /// A string value, or `None` for any other
        struct Text(Option<String>);
        impl<'de> Deserialize<'de> for Text {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Text, D::Error> {
                struct TextVisitor;
                impl<'de> Visitor<'de> for TextVisitor {
                    type Value = Option<String>;
                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("a JSON value")
                    }
                    scalars_read_empty!(Option<String>);
                    fn visit_borrowed_str<E: de::Error>(self, text: &'de str) -> Result<Option<String>, E> {
                        Ok(Some(text.to_string()))
                    }
                    fn visit_string<E: de::Error>(self, text: String) -> Result<Option<String>, E> {
                        Ok(Some(text))
                    }
                    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<String>, A::Error> {
                        skip_seq(seq).map(|()| None)
                    }
                    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Option<String>, A::Error> {
                        skip_map(map).map(|()| None)
                    }
                }
                deserializer.deserialize_any(TextVisitor).map(Text)
            }
        }
        struct EntryVisitor;
        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = Option<String>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(Option<String>);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<String>, A::Error> {
                skip_seq(seq).map(|()| None)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<String>, A::Error> {
                let mut name = None;
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Name => name = map.next_value::<Text>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(name)
            }
        }
        deserializer.deserialize_any(EntryVisitor).map(EntryName)
    }
}

/// A member list: the string `name` of each entry that has one, in order.
#[derive(Default)]
struct Names(Vec<String>);

impl<'de> Deserialize<'de> for Names {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Names, D::Error> {
        struct NamesVisitor;
        impl<'de> Visitor<'de> for NamesVisitor {
            type Value = Names;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(Names);
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Names, A::Error> {
                let mut names = Vec::new();
                while let Some(entry) = seq.next_element::<EntryName>()? {
                    names.extend(entry.0);
                }
                Ok(Names(names))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Names, A::Error> {
                skip_map(map).map(|()| Names::default())
            }
        }
        deserializer.deserialize_any(NamesVisitor)
    }
}

impl<'de> Deserialize<'de> for DumpClass {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DumpClass, D::Error> {
        struct ClassVisitor;
        impl<'de> Visitor<'de> for ClassVisitor {
            type Value = DumpClass;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(DumpClass);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<DumpClass, A::Error> {
                skip_seq(seq).map(|()| DumpClass::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<DumpClass, A::Error> {
                let mut class = DumpClass::default();
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Name => class.name = Some(map.next_value()?),
                        Key::Superclass => class.superclass = Some(map.next_value()?),
                        Key::Serialized => class.serialized = Some(map.next_value()?),
                        Key::Permits => class.permits = Some(map.next_value()?),
                        Key::Display => class.display = Some(map.next_value()?),
                        Key::Properties => class.properties = Some(map.next_value()?),
                        Key::Defaults => class.defaults = Some(map.next_value()?),
                        Key::Events => class.events = map.next_value::<Names>()?.0,
                        Key::Methods => class.methods = map.next_value::<Names>()?.0,
                        Key::Callbacks => class.callbacks = map.next_value::<Names>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(class)
            }
        }
        deserializer.deserialize_any(ClassVisitor)
    }
}

/// The entries of the dump's `classes`.
#[derive(Default)]
struct Classes(Vec<DumpClass>);

impl<'de> Deserialize<'de> for Classes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Classes, D::Error> {
        struct ClassesVisitor;
        impl<'de> Visitor<'de> for ClassesVisitor {
            type Value = Classes;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(Classes);
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Classes, A::Error> {
                let mut classes = Vec::new();
                while let Some(class) = seq.next_element::<DumpClass>()? {
                    classes.push(class);
                }
                Ok(Classes(classes))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Classes, A::Error> {
                skip_map(map).map(|()| Classes::default())
            }
        }
        deserializer.deserialize_any(ClassesVisitor)
    }
}

impl<'de> Deserialize<'de> for Dump {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Dump, D::Error> {
        struct DumpVisitor;
        impl<'de> Visitor<'de> for DumpVisitor {
            type Value = Dump;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(Dump);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Dump, A::Error> {
                skip_seq(seq).map(|()| Dump::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Dump, A::Error> {
                let mut dump = Dump::default();
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Version => dump.version = Some(map.next_value()?),
                        Key::Enums => dump.enums = Some(map.next_value()?),
                        Key::Classes => dump.classes = map.next_value::<Classes>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(dump)
            }
        }
        deserializer.deserialize_any(DumpVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflection::K;

    fn names(parameters: &[ParameterInfo]) -> Vec<(&str, &str, Option<String>)> {
        parameters.iter().map(|parameter| (&*parameter.name, &*parameter.type_info.script, parameter.default_text())).collect()
    }

    #[test]
    fn a_property_reads_its_category_permits_and_deprecation() {
        let reflection = Reflection::shared().unwrap();
        let part = reflection.class_id("Part").unwrap();
        let name = |name: &str| reflection.name_id(name).unwrap();
        let anchored = reflection.class_property(part, name("Anchored")).unwrap();
        assert_eq!((anchored.category(), anchored.deprecation_message(), anchored.is_deprecated()), (Some("Part"), None, false));
        assert_eq!(anchored.origin, MemberOrigin::Dump);
        assert_eq!(reflection.class(anchored.owner).name.as_ref(), "BasePart");
        assert!(anchored.permits.allows("Read") && anchored.permits.allows("Write"));
        assert_eq!(anchored.permits.get("Read"), Some(""));
        assert_eq!(reflection.class_property(part, K::Position as NameId).unwrap().category(), Some("Transform"));
        // a deprecated property: its message, and its category still
        let brick_color = reflection.class_property(part, name("brickColor")).unwrap();
        assert_eq!(brick_color.deprecation_message(), Some("Use `BrickColor` instead."));
        assert_eq!(brick_color.category(), Some("Appearance"));
        assert!(brick_color.is_deprecated());
        // the class an Instance property names, and a read-only one's permits
        let model = reflection.class_id("Model").unwrap();
        assert_eq!(reflection.class_property(model, name("PrimaryPart")).unwrap().instance_type.as_deref(), Some("BasePart"));
        let absolute = reflection.class_property(reflection.class_id("Frame").unwrap(), K::AbsoluteSize as NameId).unwrap();
        assert!(absolute.permits.allows("Read") && !absolute.permits.allows("Write") && absolute.read_only);
        // every property of the dump has a category; the others have none
        for property in &reflection.properties {
            assert_eq!(property.category().is_some(), property.origin == MemberOrigin::Dump, "{}", reflection.name(property.name));
        }
        let deprecated = reflection.properties.iter().filter(|property| property.is_deprecated()).count();
        assert_eq!(deprecated, 135);
        // rbx-dom's classes and the engine's own properties say where they come from
        let player = reflection.class_id("Player").unwrap();
        assert_eq!(reflection.class_property(player, K::UserId as NameId).unwrap().origin, MemberOrigin::RbxDom);
        let workspace = reflection.class_id("Workspace").unwrap();
        let streaming = reflection.class_property(workspace, name("StreamingMinRadius")).unwrap();
        assert_eq!((streaming.origin, streaming.category()), (MemberOrigin::Engine, None));
        // classes have display data and permits too
        let glue = reflection.class_by_name("Glue").unwrap();
        assert_eq!((glue.category(), glue.deprecation_message()), (Some("General"), Some("Use `WeldConstraint` instead.")));
        assert_eq!(glue.permits.get("New"), Some("Physics"));
        assert!(reflection.class_by_name("Workspace").unwrap().permits.allows("GetService"));
    }

    #[test]
    fn a_method_reads_its_parameters_returns_and_security() {
        let reflection = Reflection::shared().unwrap();
        let part = reflection.class_id("Part").unwrap();
        let instance = reflection.class_id("Instance").unwrap();
        let find = reflection.class_method(part, K::FindFirstChild as NameId).unwrap();
        assert_eq!(find.owner, instance);
        assert_eq!(names(&find.parameters), [("name", "string", None), ("recursive", "boolean", Some("false".into()))]);
        assert_eq!(find.parameters[1].type_info.engine.as_ref(), "bool");
        let returns = find.returns.as_ref().unwrap();
        assert_eq!((&*returns.script, returns.instance_type.as_deref()), ("Instance", Some("Instance")));
        assert!(find.permits.allows("Call") && find.callable_in_parallel());
        assert_eq!(find.permits.capabilities("Call").count(), 0);
        assert_eq!(reflection.parameter_default(&find.parameters[1]).and_then(|value| value.as_bool()), Some(false));
        // a method a sandboxed script needs a capability for, and that does not run in parallel
        let players = reflection.class_id("Players").unwrap();
        let user_id = reflection.class_method(players, reflection.name_id("GetUserIdFromNameAsync").unwrap()).unwrap();
        assert_eq!(user_id.permits.get("Call"), Some("Players"));
        assert_eq!(user_id.permits.capabilities("Call").collect::<Vec<_>>(), ["Players"]);
        assert!(!user_id.callable_in_parallel());
        let returns = user_id.returns.as_ref().unwrap();
        assert_eq!((&*returns.script, &*returns.engine), ("number", "int64"));
        assert_eq!(names(&user_id.parameters), [("userName", "string", None)]);
        // a deprecated method, a nil-able result, an encoded default
        let children = reflection.class_method(part, K::children as NameId).unwrap();
        assert_eq!(children.deprecation_message(), Some("Use `GetChildren` instead."));
        let raycast = reflection.class_method(reflection.class_id("Workspace").unwrap(), reflection.name_id("Raycast").unwrap()).unwrap();
        assert_eq!(raycast.returns.as_ref().unwrap().script.as_ref(), "RaycastResult?");
        assert_eq!(raycast.parameters[2].default.as_ref().and_then(|value| value.get("$t")).and_then(|kind| kind.as_str()), Some("RaycastParams"));
        let outfits = reflection.class_method(reflection.class_id("AvatarEditorService").unwrap(), reflection.name_id("GetOutfitsAsync").unwrap()).unwrap();
        assert!(outfits.parameters.iter().any(|parameter| parameter.default_text().as_deref() == Some("Enum.OutfitType.All")));
        // a method the engine adds (the dump does not list UserGameSettings) has no signature
        let settings = reflection.class_id("UserGameSettings").unwrap();
        let in_studio = reflection.class_method(settings, reflection.name_id("InStudioMode").unwrap()).unwrap();
        assert_eq!((in_studio.origin, in_studio.returns.is_none(), in_studio.parameters.len()), (MemberOrigin::Engine, true, 0));
        // a method's parameters' types name classes
        let deprecated = reflection.all_classes().map(|class| reflection.own_methods(class.id).iter().filter(|method| method.is_deprecated()).count()).sum::<usize>();
        assert_eq!(deprecated, 223);
    }

    #[test]
    fn an_event_reads_its_parameters_and_permits() {
        let reflection = Reflection::shared().unwrap();
        let part = reflection.class_id("Part").unwrap();
        let touched = reflection.class_event(part, reflection.name_id("Touched").unwrap()).unwrap();
        assert_eq!(reflection.class(touched.owner).name.as_ref(), "BasePart");
        assert_eq!(names(&touched.parameters), [("otherPart", "Instance", None)]);
        assert_eq!(touched.parameters[0].type_info.instance_type.as_deref(), Some("BasePart"));
        assert_eq!(touched.permits.get("Listen"), Some(""));
        let players = reflection.class_id("Players").unwrap();
        let added = reflection.class_event(players, reflection.name_id("PlayerAdded").unwrap()).unwrap();
        assert_eq!(added.parameters[0].type_info.instance_type.as_deref(), Some("Player"));
        assert_eq!(added.permits.get("Listen"), Some("Players"));
        let child_added = reflection.class_event(part, reflection.name_id("childAdded").unwrap()).unwrap();
        assert_eq!(child_added.deprecation_message(), Some("Use `ChildAdded` instead."));
        // a value object's Changed passes its value: its own event, not Object's
        let bool_value = reflection.class_id("BoolValue").unwrap();
        let changed = reflection.class_event(bool_value, K::Changed as NameId).unwrap();
        assert_eq!((reflection.class(changed.owner).name.as_ref(), names(&changed.parameters)), ("BoolValue", vec![("value", "boolean", None)]));
        let changed = reflection.class_event(part, K::Changed as NameId).unwrap();
        assert_eq!((reflection.class(changed.owner).name.as_ref(), &*changed.parameters[0].name), ("Object", "property"));
        // the engine's own events (Player's) and callbacks have no signature
        let player = reflection.class_id("Player").unwrap();
        assert_eq!(reflection.class_event(player, reflection.name_id("CharacterAdded").unwrap()).unwrap().origin, MemberOrigin::Engine);
        let invoke = reflection.class_callback(reflection.class_id("BindableFunction").unwrap(), K::OnInvoke as NameId).unwrap();
        assert_eq!((invoke.origin, invoke.returns.is_none()), (MemberOrigin::Engine, true));
    }

    #[test]
    fn members_are_found_through_superclasses_in_a_stable_order() {
        let reflection = Reflection::shared().unwrap();
        let part = reflection.class_id("Part").unwrap();
        // a member by class and name, inherited ones included, and of each kind
        let find = reflection.member_by_name("Part", "FindFirstChild").unwrap();
        assert_eq!((find.kind(), reflection.class(find.owner()).name.as_ref()), (MemberKind::Method, "Instance"));
        assert_eq!(reflection.member_by_name("Part", "Anchored").unwrap().kind(), MemberKind::Property);
        assert_eq!(reflection.member_by_name("Part", "Touched").unwrap().kind(), MemberKind::Event);
        assert_eq!(reflection.member_by_name("BindableFunction", "OnInvoke").unwrap().kind(), MemberKind::Callback);
        assert!(reflection.member_by_name("Part", "Nope").is_none() && reflection.member_by_name("Part", "PlayerAdded").is_none());
        // the order: class by class from the root, each class's own members in the dump's order
        let chain = reflection.class_chain(part);
        assert_eq!(chain.iter().map(|&id| &*reflection.class(id).name).collect::<Vec<_>>(), ["Object", "Instance", "PVInstance", "BasePart", "FormFactorPart", "Part"]);
        let listed: Vec<ReflectedMember> = reflection.members(part).collect();
        let depth = |member: &ReflectedMember| chain.iter().position(|&id| id == member.owner()).unwrap();
        assert!(listed.windows(2).all(|pair| depth(&pair[0]) <= depth(&pair[1])));
        let instance = reflection.class_id("Instance").unwrap();
        let own: Vec<ReflectedMember> = reflection.own_members(instance).collect();
        let inherited: Vec<ReflectedMember> = listed.iter().copied().filter(|member| member.owner() == instance).collect();
        assert_eq!(inherited, own);
        assert_eq!(listed, reflection.members(part).collect::<Vec<_>>());
        let properties: Vec<&str> = reflection.own_properties(instance).map(|property| reflection.name(property.name)).collect();
        assert_eq!(&properties[..5], ["Archivable", "Capabilities", "IsInSandbox", "Name", "Parent"]);
        // a redeclared member is listed once, where its class lists it
        let bool_value = reflection.class_id("BoolValue").unwrap();
        let changed: Vec<ReflectedMember> =
            reflection.members(bool_value).filter(|member| member.kind() == MemberKind::Event && member.name() == K::Changed as NameId).collect();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].owner(), bool_value);
        // every class's list is the members the engine resolves for it, once each
        for class in reflection.all_classes() {
            let mut seen = FxHashMap::default();
            for member in reflection.members(class.id) {
                let known = match member {
                    ReflectedMember::Property(property) => class.properties.get(&property.name).map(|&index| reflection.property(index)).is_some_and(|found| ptr::eq(found, property)),
                    ReflectedMember::Method(method) => class.methods.contains(&method.name),
                    ReflectedMember::Event(event) => class.events.contains(&event.name),
                    ReflectedMember::Callback(callback) => class.callbacks.contains(&callback.name),
                };
                assert!(known, "{}.{}", class.name, reflection.name(member.name()));
                assert!(seen.insert((member.kind(), member.name()), ()).is_none(), "{}.{} twice", class.name, reflection.name(member.name()));
            }
            assert_eq!(seen.len(), class.properties.len() + class.methods.len() + class.events.len() + class.callbacks.len(), "{}", class.name);
        }
    }

    #[test]
    fn enums_list_their_items_and_legacy_names() {
        let reflection = Reflection::shared().unwrap();
        let font = reflection.enum_by_name("Font").unwrap();
        let medium = reflection.enum_item("Font", "GothamMedium").unwrap();
        let mut legacy: Vec<&str> = reflection.legacy_names(medium).collect();
        legacy.sort_unstable();
        assert_eq!(legacy, ["GothamSemibold", "MontserratMedium"]);
        assert!(reflection.enum_items_of(font.id).all(|item| reflection.legacy_names(item.id).all(|name| name != &*item.name)));
        assert!(reflection.enum_items_of(font.id).any(|item| item.id == medium));
        assert_eq!(reflection.all_enums().count(), 662);
        for (index, item) in reflection.enum_items.iter().enumerate() {
            assert_eq!(item.id as usize, index);
        }
    }

    #[test]
    fn rbx_dom_tags_and_serialization_are_read() {
        let reflection = Reflection::shared().unwrap();
        assert_eq!(reflection.dom_version().map(|version| version[1]), Some(700));
        let workspace = reflection.dom_class_tags(reflection.class_id("Workspace").unwrap()).unwrap();
        assert!(workspace.service && workspace.not_creatable);
        let part = reflection.class_id("Part").unwrap();
        let size = reflection.dom_property(reflection.class_property(part, K::Size as NameId).unwrap()).unwrap();
        assert_eq!((size.class, size.serialization, size.data_type.as_deref()), ("BasePart", Serialization::SerializesAs("size"), Some("Vector3")));
        let position = reflection.dom_property(reflection.class_property(part, K::Position as NameId).unwrap()).unwrap();
        assert_eq!(position.serialization, Serialization::DoesNotSerialize);
        assert!(position.tags.not_replicated);
        let base_part = reflection.class_id("BasePart").unwrap();
        assert!(reflection.dom_properties(base_part).iter().any(|property| property.name == "size"));
        assert!(reflection.dom_class_tags(reflection.class_id("Player").unwrap()).is_some());
    }

    #[test]
    fn defaults_are_read_as_the_engine_gives_them() {
        let reflection = Reflection::shared().unwrap();
        let part = reflection.class_id("Part").unwrap();
        assert_eq!(reflection.default_value(part, reflection.name_id("Anchored").unwrap()).and_then(|value| value.as_bool()), Some(false));
        assert!(reflection.default_value(part, K::Size as NameId).is_some());
    }

    #[test]
    fn member_info_is_built_on_first_use() {
        let reflection = Reflection::load().unwrap();
        assert!(reflection.members.get().is_none());
        let part = reflection.class_id("Part").unwrap();
        assert!(reflection.class_method(part, K::Destroy as NameId).is_some());
        assert!(reflection.members.get().is_some());
        // from_dump builds it at once, the same
        let read = Reflection::from_dump(crate::data::REFLECTION, None).unwrap();
        assert!(read.members.get().is_some());
        for class in reflection.all_classes() {
            assert_eq!(read.own_methods(class.id), reflection.own_methods(class.id));
            assert_eq!(read.own_events(class.id), reflection.own_events(class.id));
            assert_eq!(read.own_callbacks(class.id), reflection.own_callbacks(class.id));
        }
    }

    /// What the load read from a dump when it held the whole file as JSON values: each class's name and its member lists'
    /// names, by the same helpers.
    fn read_as_values(bytes: &[u8]) -> Vec<(Option<String>, Vec<String>, Vec<String>, Vec<String>, String)> {
        let dump: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        list(dump.get("classes"))
            .iter()
            .map(|entry| {
                let names = |key: &str| list(entry.get(key)).iter().filter_map(|e| text(e.get("name"))).map(str::to_string).collect::<Vec<_>>();
                let rest = ["superclass", "serialized", "permits", "display", "properties", "defaults"].map(|key| format!("{:?}", entry.get(key))).join(";");
                (text(entry.get("name")).map(str::to_string), names("events"), names("methods"), names("callbacks"), rest)
            })
            .collect()
    }

    fn read_as_dump(bytes: &[u8]) -> Vec<(Option<String>, Vec<String>, Vec<String>, Vec<String>, String)> {
        let dump: Dump = serde_json::from_slice(bytes).unwrap();
        dump.classes
            .into_iter()
            .map(|class| {
                let rest = [&class.superclass, &class.serialized, &class.permits, &class.display, &class.properties, &class.defaults].map(|value| format!("{:?}", value.as_ref())).join(";");
                (text(class.name.as_ref()).map(str::to_string), class.events, class.methods, class.callbacks, rest)
            })
            .collect()
    }

    #[test]
    fn the_load_reads_what_the_json_values_held() {
        assert_eq!(read_as_dump(crate::data::REFLECTION), read_as_values(crate::data::REFLECTION));
        // entries of other shapes: not an object, lists that are `{}` or no list, names that are no string, repeated and
        // escaped keys, nesting
        let odd = br#"{"version": "x", "classes": [
            5, [1, {"name": "A"}], null, "Part",
            {"name": "A", "events": {}, "methods": [{"name": "M", "parameters": [{"name": "p", "default": {"$t": "table", "v": {"name": "no"}}}]}, {"name": 3}, 7, [], {"name": "Escaped"}, {"other": {"name": "deep"}}], "callbacks": "none"},
            {"name": 5, "methods": [{"name": "Lost"}]},
            {"name": "B", "name": "C", "methods": [{"name": "X", "name": "Y"}, {"name": "Z", "name": null}], "methods": [{"name": "Last"}], "events": [{"name": "E"}], "superclass": "A", "defaults": {"Size": {"$t": "Vector3", "v": [1, 2, 3]}}}
        ], "enums": []}"#;
        assert_eq!(read_as_dump(odd), read_as_values(odd));
        for top in [&br#"[]"#[..], br#"5"#, br#"{"classes": {}}"#, br#"{"classes": 3}"#, br#"{}"#] {
            assert_eq!(read_as_dump(top), read_as_values(top));
        }
    }
}
