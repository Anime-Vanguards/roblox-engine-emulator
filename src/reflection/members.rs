//! What the reflection knows about members beyond what the engine runs on: methods, events and callbacks with their
//! parameters, types, permits and display data, rbx-dom's tags and serialization, and the lookups an editor needs
//! (re-exported from [`crate::reflection`], whose module documentation has an example).
//!
//! The engine itself reads none of this. A method's, event's or callback's info is built on the first call that asks for
//! one (the compiled-in dump is read again for it, some milliseconds), so an engine that never asks boots as it did.

use std::{borrow::Cow, fmt, marker::PhantomData, ptr};

use rbx_dom_weak::types::{self as dom, Variant};
use rustc_hash::FxHashMap;
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use super::{
    BEHAVIOR_METHODS, BEHAVIOR_PROPERTIES, ClassId, ClassInfo, DumpEnumItem, EnumId, EnumInfo, EnumItemId, EnumItemInfo, ITEM_ALIASES, NameId,
    PropertyInfo, Reflection, ScriptType, engine_type, list, text,
};
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
    /// The engine's own lists: a member the dump leaves out that the engine has (measured, or documented by Roblox), such as
    /// Player's methods (Kick, GetMouse, LoadCharacter), scripts' Source and BindableFunction.OnInvoke. Its name and class are
    /// known, and a property's type; a method's, event's or callback's parameters and types are not (ree records none: its
    /// `parameters` are empty and its `returns` is `None`), and it has no permits or display data.
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
        crate::sort::by(&mut permits, |a, b| a.0.cmp(&b.0));
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

#[inline(never)]
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
                crate::sort::ord(&mut parts);
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

/// The methods, events and callbacks of every class, and the properties it has only through behaviours, built on first use
/// ([`Reflection::members`]).
#[derive(Debug, Default)]
pub(super) struct Members {
    methods: Vec<MethodInfo>,
    events: Vec<EventInfo>,
    callbacks: Vec<CallbackInfo>,
    /// [`BEHAVIOR_PROPERTIES`] as properties ([`MemberOrigin::Engine`])
    properties: Vec<PropertyInfo>,
    /// Per class: where its own methods, events, callbacks and behaviour properties are in the lists above (start, end), in
    /// its order
    own: Vec<[(u32, u32); 4]>,
    /// (declaring class, name) to the index in each list
    declared: [FxHashMap<(ClassId, NameId), u32>; 4],
}

const METHODS: usize = 0;
const EVENTS: usize = 1;
const CALLBACKS: usize = 2;
const PROPERTIES: usize = 3;

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
    /// The members of every class of `reflection`, from the dump it was read from: its own lists give each class's
    /// members and order (then the methods and properties it has only through behaviours), the dump's entry the rest (an
    /// engine member has none). The load made class n of the dump's n-th entry with a name, so entries are matched by place.
    #[inline(never)]
    pub(super) fn build(reflection: &Reflection, dump: &[u8]) -> Members {
        let classes = serde_json::from_slice::<MemberDump>(dump).map(|dump| dump.classes).unwrap_or_default();
        let entries: Vec<&MemberClass> = classes.iter().filter(|class| text(class.name.as_ref()).is_some()).collect();
        let mut members = Members::default();
        for class in &reflection.classes {
            let entry = if class.reflected { entries.get(class.id as usize).copied() } else { None };
            let parameters = |member: Option<&serde_json::Value>| -> Box<[ParameterInfo]> {
                list(member.and_then(|member| member.get("parameters"))).iter().map(ParameterInfo::from_json).collect()
            };
            let permits = |member: Option<&serde_json::Value>| Permits::from_json(member.and_then(|member| member.get("permits")));
            let display = |member: Option<&serde_json::Value>| display_from_json(member.and_then(|member| member.get("display")));
            let origin = |member: Option<&serde_json::Value>| if member.is_some() { MemberOrigin::Dump } else { MemberOrigin::Engine };
            let mut own = [(0u32, 0u32); 4];

            let dumped = by_name(entry.and_then(|entry| entry.methods.as_ref()));
            own[METHODS].0 = members.methods.len() as u32;
            let behaviour_methods = BEHAVIOR_METHODS
                .iter()
                .filter(|(owner, _)| *owner == &*class.name)
                .flat_map(|(_, names)| names.iter())
                .filter_map(|name| reflection.name_id(name))
                .filter(|name| !class.own_methods.contains(name));
            for name in class.own_methods.iter().copied().chain(behaviour_methods) {
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

            own[PROPERTIES].0 = members.properties.len() as u32;
            for &(owner, name, script_type, engine_type_name, read_only, serialized) in BEHAVIOR_PROPERTIES {
                let Some(name) = reflection.name_id(name).filter(|&name| owner == &*class.name && reflection.class_property(class.id, name).is_none())
                else {
                    continue;
                };
                members.declared[PROPERTIES].entry((class.id, name)).or_insert(members.properties.len() as u32);
                members.properties.push(PropertyInfo {
                    name,
                    script_type: ScriptType::parse(script_type),
                    script_type_name: script_type.into(),
                    engine_type: engine_type(engine_type_name),
                    engine_type_name: engine_type_name.into(),
                    enum_name: None,
                    enum_type: None,
                    read_only,
                    scriptable: true,
                    serialized,
                    owner: class.id,
                    instance_type: None,
                    display: Box::new([]),
                    permits: Permits::default(),
                    origin: MemberOrigin::Engine,
                });
            }
            own[PROPERTIES].1 = members.properties.len() as u32;
            members.own.push(own);
        }
        members
    }
}

impl Reflection {
    /// The methods, events and callbacks, built on the first call.
    fn member_store(&self) -> &Members {
        self.members.get_or_init(|| self.member_source.0.map(|dump| Members::build(self, dump)).unwrap_or_default())
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
        let enum_info = &self.enums[info.enum_type as usize];
        ITEM_ALIASES
            .iter()
            .filter(move |(name, _)| *name == &*enum_info.name)
            .flat_map(|(_, aliases)| aliases.iter())
            // (a legacy name the load installed for this item: not one an item of the enum already has)
            .filter(move |(alias, _)| enum_info.by_name.get(*alias) == Some(&item))
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

    /// The properties `class` declares, in the dump's order (then the ones the engine adds, and those it has only through
    /// behaviours).
    pub fn own_properties(&self, class: ClassId) -> impl Iterator<Item = &PropertyInfo> {
        let (start, end) = self.member_store().own[class as usize][PROPERTIES];
        self.classes[class as usize]
            .own_properties
            .iter()
            .map(|&index| &self.properties[index as usize])
            .chain(self.member_store().properties[start as usize..end as usize].iter())
    }

    /// The methods `class` declares, in the dump's order (then the ones the engine adds, and those it has only through
    /// behaviours: Player's).
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
    /// `class` itself ([`Self::class_chain`]: Object's first, then Instance's), each class's as [`Self::own_members`] lists
    /// them. A member a class declares again (a value object's Changed, CollectionService's AddTag) is listed once, where
    /// that class lists it, as [`Self::member`] finds it.
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

    /// The method `name` of `class`, its own or inherited (the nearest class's that declares it), the ones it has only through
    /// behaviours included.
    pub fn class_method(&self, class: ClassId, name: NameId) -> Option<&MethodInfo> {
        self.declared(METHODS, &self.member_store().methods, class, name)
    }

    /// The event `name` of `class`, its own or inherited (the nearest class's that declares it).
    pub fn class_event(&self, class: ClassId, name: NameId) -> Option<&EventInfo> {
        self.declared(EVENTS, &self.member_store().events, class, name)
    }

    /// The callback `name` of `class`, its own or inherited.
    pub fn class_callback(&self, class: ClassId, name: NameId) -> Option<&CallbackInfo> {
        self.declared(CALLBACKS, &self.member_store().callbacks, class, name)
    }

    fn member_of_kind(&self, class: ClassId, kind: MemberKind, name: NameId) -> Option<ReflectedMember<'_>> {
        match kind {
            MemberKind::Property => self
                .class_property(class, name)
                .or_else(|| self.declared(PROPERTIES, &self.member_store().properties, class, name))
                .map(ReflectedMember::Property),
            MemberKind::Method => self.class_method(class, name).map(ReflectedMember::Method),
            MemberKind::Event => self.class_event(class, name).map(ReflectedMember::Event),
            MemberKind::Callback => self.class_callback(class, name).map(ReflectedMember::Callback),
        }
    }

    /// The member `name` of `class`, its own or inherited, of any kind: a property, else a method, an event or a callback.
    /// The ones the class has only through the engine's behaviours are found too (Player's Kick, scripts' Source). Unlike an
    /// instance's member read, this finds a property scripts cannot read as well (PVInstance.Origin, where a script's
    /// `part.Origin` finds a child named Origin: [`PropertyInfo::scriptable`] says which), and a deprecated lower-case member
    /// as itself (`brickColor`, which a script's read takes for BrickColor: [`Self::property_alias`], [`Self::method_alias`]).
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
    #[inline(never)]
    pub fn dom_properties(&self, class: ClassId) -> Vec<DomProperty> {
        let Some(descriptor) = rbx_reflection_database::get().ok().and_then(|database| database.classes.get(&*self.classes[class as usize].name)) else {
            return Vec::new();
        };
        let mut properties: Vec<DomProperty> =
            descriptor.properties.iter().map(|(name, property)| dom_property(&descriptor.name, name, property)).collect();
        crate::sort::by(&mut properties, |a, b| a.name.cmp(b.name));
        properties
    }
}

// The dump as the engine reads it at load. Every field is streamed directly into typed structures without building
// intermediate `serde_json::Value` trees or `BTreeMap`s. Any shape of JSON reads as the JSON values did: a value of
// another type than expected reads as empty, a repeated key's last value counts, an entry without a string name is
// skipped, and a string reads the same with escapes as without. The member info built on first use ([`Members::build`])
// reads the dump by the same rules.

/// An enum entry of the dump (`enums` array).
pub(super) struct DumpEnum {
    pub name: String,
    pub items: Vec<DumpEnumItem>,
}

/// A permits object of the dump, together with the presence of the keys the engine checks on raw permits.
#[derive(Default)]
pub(super) struct DumpPermits {
    pub permits: Permits,
    pub has_new: bool,
    pub has_get_service: bool,
    pub has_read: bool,
    pub has_write: bool,
}

/// A property entry of the dump (`properties` array).
pub(super) struct DumpProperty {
    pub name: String,
    pub script: Option<String>,
    pub engine: Option<String>,
    pub enum_name: Option<String>,
    pub instance_type: Option<String>,
    pub read_only: bool,
    pub scriptable: bool,
    pub serialized: bool,
    pub display: DisplayData,
    pub permits: Permits,
}

/// A default entry of the dump (`defaults` object), decoded directly into its typed form without `serde_json::Value`.
pub(super) enum ParsedDefault {
    Dom(Variant),
    EnumItem(String, String),
    Font(String, String, String),
    None,
}

/// A class entry of the dump: the fields the load reads, directly typed, and the member lists' names.
#[derive(Default)]
pub(super) struct DumpClass {
    pub name: Option<String>,
    pub superclass: Option<String>,
    pub serialized: bool,
    pub permits: DumpPermits,
    pub display: DisplayData,
    pub properties: Vec<DumpProperty>,
    pub defaults: Option<Vec<(String, ParsedDefault)>>,
    /// The `name` of each entry of `events`, `methods` and `callbacks` that has a string one, in order
    pub events: Vec<String>,
    pub methods: Vec<String>,
    pub callbacks: Vec<String>,
}

/// The dump's top level as the initial load reads it (an entry that is no object reads as one without a name).
#[derive(Default)]
pub(super) struct DumpLoad {
    pub version: Option<String>,
    pub enums: Vec<DumpEnum>,
    pub classes: Vec<DumpClass>,
}

/// The dump's top level as the lazy member info reads it (skips `version` and `enums`).
#[derive(Default)]
struct MemberDump {
    classes: Vec<MemberClass>,
}

/// A class entry as the member info is built from it: its name and its member lists as JSON values (the rest skipped).
#[derive(Default)]
struct MemberClass {
    name: Option<serde_json::Value>,
    events: Option<serde_json::Value>,
    methods: Option<serde_json::Value>,
    callbacks: Option<serde_json::Value>,
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
    Items,
    Value,
    Type,
    Script,
    Engine,
    Enum,
    Instance,
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
                    "items" => Key::Items,
                    "value" => Key::Value,
                    "type" => Key::Type,
                    "script" => Key::Script,
                    "engine" => Key::Engine,
                    "enum" => Key::Enum,
                    "instance" => Key::Instance,
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

/// A string value, or `None` for any other JSON value.
struct OptText(Option<String>);

impl<'de> Deserialize<'de> for OptText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<OptText, D::Error> {
        struct TextVisitor;
        impl<'de> Visitor<'de> for TextVisitor {
            type Value = Option<String>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<Option<String>, E> {
                Ok(Some(text.to_string()))
            }
            fn visit_string<E: de::Error>(self, text: String) -> Result<Option<String>, E> {
                Ok(Some(text))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Option<String>, E> {
                Ok(None)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Option<String>, E> {
                Ok(None)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Option<String>, E> {
                Ok(None)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Option<String>, E> {
                Ok(None)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Option<String>, E> {
                Ok(None)
            }
            fn visit_none<E: de::Error>(self) -> Result<Option<String>, E> {
                Ok(None)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<String>, A::Error> {
                skip_seq(seq).map(|()| None)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Option<String>, A::Error> {
                skip_map(map).map(|()| None)
            }
        }
        deserializer.deserialize_any(TextVisitor).map(OptText)
    }
}

/// A boolean `true`, or `false` for any other JSON value.
struct IsTrue(bool);

impl<'de> Deserialize<'de> for IsTrue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<IsTrue, D::Error> {
        struct BoolVisitor;
        impl<'de> Visitor<'de> for BoolVisitor {
            type Value = bool;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<bool, E> {
                Ok(v)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<bool, E> {
                Ok(false)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<bool, E> {
                Ok(false)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<bool, E> {
                Ok(false)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<bool, E> {
                Ok(false)
            }
            fn visit_unit<E: de::Error>(self) -> Result<bool, E> {
                Ok(false)
            }
            fn visit_none<E: de::Error>(self) -> Result<bool, E> {
                Ok(false)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<bool, A::Error> {
                skip_seq(seq).map(|()| false)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<bool, A::Error> {
                skip_map(map).map(|()| false)
            }
        }
        deserializer.deserialize_any(BoolVisitor).map(IsTrue)
    }
}

/// A JSON number as `f64`, or `None` for any other JSON value.
struct OptF64(Option<f64>);

impl<'de> Deserialize<'de> for OptF64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<OptF64, D::Error> {
        struct NumVisitor;
        impl<'de> Visitor<'de> for NumVisitor {
            type Value = Option<f64>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Option<f64>, E> {
                Ok(Some(v as f64))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Option<f64>, E> {
                Ok(Some(v as f64))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Option<f64>, E> {
                Ok(Some(v))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Option<f64>, E> {
                Ok(None)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Option<f64>, E> {
                Ok(None)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Option<f64>, E> {
                Ok(None)
            }
            fn visit_none<E: de::Error>(self) -> Result<Option<f64>, E> {
                Ok(None)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<f64>, A::Error> {
                skip_seq(seq).map(|()| None)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Option<f64>, A::Error> {
                skip_map(map).map(|()| None)
            }
        }
        deserializer.deserialize_any(NumVisitor).map(OptF64)
    }
}

/// A JSON `display` object read directly into [`DisplayData`], preserving sorted key order and last-key-wins semantics.
#[derive(Default)]
struct ParsedDisplay(DisplayData);

impl<'de> Deserialize<'de> for ParsedDisplay {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ParsedDisplay, D::Error> {
        struct DisplayVisitor;
        impl<'de> Visitor<'de> for DisplayVisitor {
            type Value = ParsedDisplay;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(ParsedDisplay);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<ParsedDisplay, A::Error> {
                skip_seq(seq).map(|()| ParsedDisplay::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ParsedDisplay, A::Error> {
                let mut entries: Vec<(Box<str>, Box<str>)> = Vec::new();
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    let value = map.next_value::<OptText>()?.0;
                    if let Some(pos) = entries.iter().position(|(k, _)| &**k == &*key) {
                        match value {
                            Some(text) => entries[pos].1 = text.into_boxed_str(),
                            None => {
                                entries.remove(pos);
                            }
                        }
                    } else if let Some(text) = value {
                        entries.push((key.into_owned().into_boxed_str(), text.into_boxed_str()));
                    }
                }
                if entries.len() > 1 {
                    crate::sort::by(&mut entries, |a, b| a.0.cmp(&b.0));
                }
                Ok(ParsedDisplay(entries.into_boxed_slice()))
            }
        }
        deserializer.deserialize_any(DisplayVisitor)
    }
}

impl<'de> Deserialize<'de> for DumpPermits {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DumpPermits, D::Error> {
        struct PermitsVisitor;
        impl<'de> Visitor<'de> for PermitsVisitor {
            type Value = DumpPermits;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(DumpPermits);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<DumpPermits, A::Error> {
                skip_seq(seq).map(|()| DumpPermits::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<DumpPermits, A::Error> {
                let mut out = DumpPermits::default();
                let mut entries: Vec<(Cow<'static, str>, Box<str>)> = Vec::new();
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match &*key {
                        "New" => out.has_new = true,
                        "GetService" => out.has_get_service = true,
                        "Read" => out.has_read = true,
                        "Write" => out.has_write = true,
                        _ => {}
                    }
                    let value = map.next_value::<OptText>()?.0;
                    if let Some(pos) = entries.iter().position(|(k, _)| &**k == &*key) {
                        match value {
                            Some(text) => entries[pos].1 = text.into_boxed_str(),
                            None => {
                                entries.remove(pos);
                            }
                        }
                    } else if let Some(text) = value {
                        let action = match ACTIONS.iter().find(|known| **known == &*key) {
                            Some(known) => Cow::Borrowed(*known),
                            None => Cow::Owned(key.into_owned()),
                        };
                        entries.push((action, text.into_boxed_str()));
                    }
                }
                if entries.len() > 1 {
                    crate::sort::by(&mut entries, |a, b| a.0.cmp(&b.0));
                }
                out.permits = Permits(entries.into_boxed_slice());
                Ok(out)
            }
        }
        deserializer.deserialize_any(PermitsVisitor)
    }
}

/// An enum item entry (`{"name": "...", "value": 0}`), or `None` when malformed.
struct EnumItemEntry(Option<DumpEnumItem>);

impl<'de> Deserialize<'de> for EnumItemEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<EnumItemEntry, D::Error> {
        struct ItemVisitor;
        impl<'de> Visitor<'de> for ItemVisitor {
            type Value = Option<DumpEnumItem>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(Option<DumpEnumItem>);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<DumpEnumItem>, A::Error> {
                skip_seq(seq).map(|()| None)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<DumpEnumItem>, A::Error> {
                let mut name = None;
                let mut value = None;
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Name => name = map.next_value::<OptText>()?.0,
                        Key::Value => value = map.next_value::<OptF64>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(match (name, value) {
                    (Some(name), Some(value)) => Some(DumpEnumItem { name, value }),
                    _ => None,
                })
            }
        }
        deserializer.deserialize_any(ItemVisitor).map(EnumItemEntry)
    }
}

#[derive(Default)]
struct EnumItems(Vec<DumpEnumItem>);

impl<'de> Deserialize<'de> for EnumItems {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<EnumItems, D::Error> {
        struct ItemsVisitor;
        impl<'de> Visitor<'de> for ItemsVisitor {
            type Value = EnumItems;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(EnumItems);
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<EnumItems, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element::<EnumItemEntry>()? {
                    items.extend(item.0);
                }
                Ok(EnumItems(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<EnumItems, A::Error> {
                skip_map(map).map(|()| EnumItems::default())
            }
        }
        deserializer.deserialize_any(ItemsVisitor)
    }
}

struct EnumEntry(Option<DumpEnum>);

impl<'de> Deserialize<'de> for EnumEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<EnumEntry, D::Error> {
        struct EnumVisitor;
        impl<'de> Visitor<'de> for EnumVisitor {
            type Value = Option<DumpEnum>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(Option<DumpEnum>);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<DumpEnum>, A::Error> {
                skip_seq(seq).map(|()| None)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<DumpEnum>, A::Error> {
                let mut name = None;
                let mut items = Vec::new();
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Name => name = map.next_value::<OptText>()?.0,
                        Key::Items => items = map.next_value::<EnumItems>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(name.map(|name| DumpEnum { name, items }))
            }
        }
        deserializer.deserialize_any(EnumVisitor).map(EnumEntry)
    }
}

#[derive(Default)]
struct EnumsList(Vec<DumpEnum>);

impl<'de> Deserialize<'de> for EnumsList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<EnumsList, D::Error> {
        struct EnumsVisitor;
        impl<'de> Visitor<'de> for EnumsVisitor {
            type Value = EnumsList;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(EnumsList);
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<EnumsList, A::Error> {
                let mut enums = Vec::with_capacity(660);
                while let Some(entry) = seq.next_element::<EnumEntry>()? {
                    enums.extend(entry.0);
                }
                Ok(EnumsList(enums))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<EnumsList, A::Error> {
                skip_map(map).map(|()| EnumsList::default())
            }
        }
        deserializer.deserialize_any(EnumsVisitor)
    }
}

#[derive(Default)]
struct PropType {
    script: Option<String>,
    engine: Option<String>,
    enum_name: Option<String>,
    instance_type: Option<String>,
}

impl<'de> Deserialize<'de> for PropType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PropType, D::Error> {
        struct PropTypeVisitor;
        impl<'de> Visitor<'de> for PropTypeVisitor {
            type Value = PropType;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(PropType);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<PropType, A::Error> {
                skip_seq(seq).map(|()| PropType::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<PropType, A::Error> {
                let mut out = PropType::default();
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Script => out.script = map.next_value::<OptText>()?.0,
                        Key::Engine => out.engine = map.next_value::<OptText>()?.0,
                        Key::Enum => out.enum_name = map.next_value::<OptText>()?.0,
                        Key::Instance => out.instance_type = map.next_value::<OptText>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
        }
        deserializer.deserialize_any(PropTypeVisitor)
    }
}

struct PropertyEntry(Option<DumpProperty>);

impl<'de> Deserialize<'de> for PropertyEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PropertyEntry, D::Error> {
        struct PropertyVisitor;
        impl<'de> Visitor<'de> for PropertyVisitor {
            type Value = Option<DumpProperty>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(Option<DumpProperty>);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<DumpProperty>, A::Error> {
                skip_seq(seq).map(|()| None)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<DumpProperty>, A::Error> {
                let mut name = None;
                let mut prop_type = PropType::default();
                let mut permits = DumpPermits::default();
                let mut display = DisplayData::default();
                let mut serialized = false;
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Name => name = map.next_value::<OptText>()?.0,
                        Key::Type => prop_type = map.next_value()?,
                        Key::Permits => permits = map.next_value()?,
                        Key::Display => display = map.next_value::<ParsedDisplay>()?.0,
                        Key::Serialized => serialized = map.next_value::<IsTrue>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(name.map(|name| DumpProperty {
                    name,
                    script: prop_type.script,
                    engine: prop_type.engine,
                    enum_name: prop_type.enum_name,
                    instance_type: prop_type.instance_type,
                    read_only: !permits.has_write,
                    scriptable: permits.has_read,
                    serialized,
                    display,
                    permits: permits.permits,
                }))
            }
        }
        deserializer.deserialize_any(PropertyVisitor).map(PropertyEntry)
    }
}

#[derive(Default)]
struct PropertiesList(Vec<DumpProperty>);

impl<'de> Deserialize<'de> for PropertiesList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PropertiesList, D::Error> {
        struct PropsVisitor;
        impl<'de> Visitor<'de> for PropsVisitor {
            type Value = PropertiesList;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(PropertiesList);
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<PropertiesList, A::Error> {
                let mut props = Vec::new();
                while let Some(prop) = seq.next_element::<PropertyEntry>()? {
                    props.extend(prop.0);
                }
                Ok(PropertiesList(props))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<PropertiesList, A::Error> {
                skip_map(map).map(|()| PropertiesList::default())
            }
        }
        deserializer.deserialize_any(PropsVisitor)
    }
}

/// Parses a numeric element inside a default array/scalar as `value::decode_number` does.
fn parse_special_number(text: &str) -> f64 {
    match text {
        "inf" | "+inf" => f64::INFINITY,
        "-inf" => f64::NEG_INFINITY,
        "nan" | "-nan" => f64::NAN,
        other => other.parse().unwrap_or(0.0),
    }
}

/// Reads a single JSON number, numeric string, or `{"t":"numeric","v":...}` / `{"s":...}` object as `f64`.
struct DefaultNum(f64);

impl<'de> Deserialize<'de> for DefaultNum {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DefaultNum, D::Error> {
        struct NumVisitor;
        impl<'de> Visitor<'de> for NumVisitor {
            type Value = f64;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<f64, E> {
                Ok(v as f64)
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<f64, E> {
                Ok(v as f64)
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<f64, E> {
                Ok(v)
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<f64, E> {
                Ok(s.parse().unwrap_or(0.0))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<f64, E> {
                Ok(0.0)
            }
            fn visit_unit<E: de::Error>(self) -> Result<f64, E> {
                Ok(0.0)
            }
            fn visit_none<E: de::Error>(self) -> Result<f64, E> {
                Ok(0.0)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<f64, A::Error> {
                skip_seq(seq).map(|()| 0.0)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<f64, A::Error> {
                let mut is_numeric = false;
                let mut v_val: Option<String> = None;
                let mut s_val: Option<String> = None;
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match &*key {
                        "t" => is_numeric = map.next_value::<OptText>()?.0.as_deref() == Some("numeric"),
                        "v" => v_val = map.next_value::<OptText>()?.0,
                        "s" => s_val = map.next_value::<OptText>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                let text = if is_numeric { v_val.as_deref() } else { s_val.as_deref() };
                Ok(text.map_or(0.0, parse_special_number))
            }
        }
        deserializer.deserialize_any(NumVisitor).map(DefaultNum)
    }
}

/// Up to 12 numbers inline (covers Vector2/3, UDim/2, Color3, Rect, NumberRange, CFrame without
/// heap allocation), spilling to a `Vec` for longer sequences.
#[derive(Default)]
struct SmallNums {
    data: [f64; 12],
    len: usize,
    spill: Option<Vec<f64>>,
}

impl SmallNums {
    #[inline]
    fn push(&mut self, value: f64) {
        if let Some(spill) = self.spill.as_mut() {
            spill.push(value);
        } else if self.len < 12 {
            self.data[self.len] = value;
        } else {
            let mut vec = Vec::with_capacity(24);
            vec.extend_from_slice(&self.data);
            vec.push(value);
            self.spill = Some(vec);
        }
        self.len += 1;
    }

    #[inline]
    fn at(&self, index: usize) -> f64 {
        if let Some(spill) = self.spill.as_ref() {
            spill.get(index).copied().unwrap_or(0.0)
        } else if index < self.len {
            self.data[index]
        } else {
            0.0
        }
    }
}

enum DefaultSeqItem {
    Num(f64),
    Str(String),
    Nested([f64; 4]),
    Other,
}

impl<'de> Deserialize<'de> for DefaultSeqItem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DefaultSeqItem, D::Error> {
        struct ItemVisitor;
        impl<'de> Visitor<'de> for ItemVisitor {
            type Value = DefaultSeqItem;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<DefaultSeqItem, E> {
                Ok(DefaultSeqItem::Num(v as f64))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<DefaultSeqItem, E> {
                Ok(DefaultSeqItem::Num(v as f64))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<DefaultSeqItem, E> {
                Ok(DefaultSeqItem::Num(v))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<DefaultSeqItem, E> {
                Ok(DefaultSeqItem::Str(s.to_string()))
            }
            fn visit_string<E: de::Error>(self, s: String) -> Result<DefaultSeqItem, E> {
                Ok(DefaultSeqItem::Str(s))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<DefaultSeqItem, E> {
                Ok(DefaultSeqItem::Other)
            }
            fn visit_unit<E: de::Error>(self) -> Result<DefaultSeqItem, E> {
                Ok(DefaultSeqItem::Other)
            }
            fn visit_none<E: de::Error>(self) -> Result<DefaultSeqItem, E> {
                Ok(DefaultSeqItem::Other)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<DefaultSeqItem, A::Error> {
                let mut row = [0.0f64; 4];
                let mut i = 0usize;
                while let Some(n) = seq.next_element::<DefaultNum>()? {
                    if i < 4 {
                        row[i] = n.0;
                    }
                    i += 1;
                }
                Ok(DefaultSeqItem::Nested(row))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<DefaultSeqItem, A::Error> {
                let mut is_numeric = false;
                let mut v_val: Option<String> = None;
                let mut s_val: Option<String> = None;
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match &*key {
                        "t" => is_numeric = map.next_value::<OptText>()?.0.as_deref() == Some("numeric"),
                        "v" => v_val = map.next_value::<OptText>()?.0,
                        "s" => s_val = map.next_value::<OptText>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                let text = if is_numeric { v_val.as_deref() } else { s_val.as_deref() };
                Ok(DefaultSeqItem::Num(text.map_or(0.0, parse_special_number)))
            }
        }
        deserializer.deserialize_any(ItemVisitor)
    }
}

enum DefaultV {
    Num(f64),
    Str(String),
    Array {
        nums: SmallNums,
        nested: Option<Vec<[f64; 4]>>,
        strings: Option<Vec<Option<String>>>,
    },
    Other,
}

impl<'de> Deserialize<'de> for DefaultV {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DefaultV, D::Error> {
        struct VVisitor;
        impl<'de> Visitor<'de> for VVisitor {
            type Value = DefaultV;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<DefaultV, E> {
                Ok(DefaultV::Num(v as f64))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<DefaultV, E> {
                Ok(DefaultV::Num(v as f64))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<DefaultV, E> {
                Ok(DefaultV::Num(v))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<DefaultV, E> {
                Ok(DefaultV::Str(s.to_string()))
            }
            fn visit_string<E: de::Error>(self, s: String) -> Result<DefaultV, E> {
                Ok(DefaultV::Str(s))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<DefaultV, E> {
                Ok(DefaultV::Other)
            }
            fn visit_unit<E: de::Error>(self) -> Result<DefaultV, E> {
                Ok(DefaultV::Other)
            }
            fn visit_none<E: de::Error>(self) -> Result<DefaultV, E> {
                Ok(DefaultV::Other)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<DefaultV, A::Error> {
                let mut nums = SmallNums::default();
                let mut nested: Option<Vec<[f64; 4]>> = None;
                let mut strings: Option<Vec<Option<String>>> = None;
                let mut count = 0usize;
                while let Some(item) = seq.next_element::<DefaultSeqItem>()? {
                    match item {
                        DefaultSeqItem::Num(n) => {
                            nums.push(n);
                            if let Some(rows) = nested.as_mut() {
                                rows.push([n, 0.0, 0.0, 0.0]);
                            }
                            if let Some(list) = strings.as_mut() {
                                list.push(None);
                            }
                        }
                        DefaultSeqItem::Str(s) => {
                            let parsed = s.parse().unwrap_or(0.0);
                            nums.push(parsed);
                            if let Some(rows) = nested.as_mut() {
                                rows.push([parsed, 0.0, 0.0, 0.0]);
                            }
                            let list = strings.get_or_insert_with(|| vec![None; count]);
                            list.push(Some(s));
                        }
                        DefaultSeqItem::Nested(row) => {
                            nums.push(0.0);
                            let rows = nested.get_or_insert_with(|| {
                                (0..count).map(|i| [nums.at(i), 0.0, 0.0, 0.0]).collect()
                            });
                            rows.push(row);
                            if let Some(list) = strings.as_mut() {
                                list.push(None);
                            }
                        }
                        DefaultSeqItem::Other => {
                            nums.push(0.0);
                            if let Some(rows) = nested.as_mut() {
                                rows.push([0.0; 4]);
                            }
                            if let Some(list) = strings.as_mut() {
                                list.push(None);
                            }
                        }
                    }
                    count += 1;
                }
                Ok(DefaultV::Array { nums, nested, strings })
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<DefaultV, A::Error> {
                let mut is_numeric = false;
                let mut v_val: Option<String> = None;
                let mut s_val: Option<String> = None;
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match &*key {
                        "t" => is_numeric = map.next_value::<OptText>()?.0.as_deref() == Some("numeric"),
                        "v" => v_val = map.next_value::<OptText>()?.0,
                        "s" => s_val = map.next_value::<OptText>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                let text = if is_numeric { v_val.as_deref() } else { s_val.as_deref() };
                Ok(DefaultV::Num(text.map_or(0.0, parse_special_number)))
            }
        }
        deserializer.deserialize_any(VVisitor)
    }
}

#[derive(Clone, Copy)]
enum DefaultTag {
    Number,
    EnumItem,
    UDim,
    UDim2,
    Vector2,
    Vector2int16,
    Vector3,
    Vector3int16,
    Color3,
    BrickColor,
    CFrame,
    Rect,
    NumberRange,
    NumberSequence,
    ColorSequence,
    Font,
    Content,
    /// Defaults the dump records as text in `s`: a part's ResizeableFaces ("Right, Top, Back, Left, Bottom, Front"), Handles.Faces,
    /// ArcHandles.Axes ("X, Y, Z"), RayValue.Value ("{0, 0, 0}, {0, 0, 0}"); a client reads them (measured, Studio 0.747, a client and
    /// a server: tools/parity/probes/instance_api_texts.luau, value_read_identity.luau)
    Faces,
    Axes,
    Ray,
    Other,
}

struct OptDefaultTag(Option<DefaultTag>);

impl<'de> Deserialize<'de> for OptDefaultTag {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<OptDefaultTag, D::Error> {
        struct TagVisitor;
        impl<'de> Visitor<'de> for TagVisitor {
            type Value = Option<DefaultTag>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Option<DefaultTag>, E> {
                Ok(Some(match s {
                    "number" => DefaultTag::Number,
                    "EnumItem" => DefaultTag::EnumItem,
                    "UDim" => DefaultTag::UDim,
                    "UDim2" => DefaultTag::UDim2,
                    "Vector2" => DefaultTag::Vector2,
                    "Vector2int16" => DefaultTag::Vector2int16,
                    "Vector3" => DefaultTag::Vector3,
                    "Vector3int16" => DefaultTag::Vector3int16,
                    "Color3" => DefaultTag::Color3,
                    "BrickColor" => DefaultTag::BrickColor,
                    "CFrame" => DefaultTag::CFrame,
                    "Rect" => DefaultTag::Rect,
                    "NumberRange" => DefaultTag::NumberRange,
                    "NumberSequence" => DefaultTag::NumberSequence,
                    "ColorSequence" => DefaultTag::ColorSequence,
                    "Font" => DefaultTag::Font,
                    "Content" => DefaultTag::Content,
                    "Faces" => DefaultTag::Faces,
                    "Axes" => DefaultTag::Axes,
                    "Ray" => DefaultTag::Ray,
                    _ => DefaultTag::Other,
                }))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Option<DefaultTag>, E> {
                Ok(None)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Option<DefaultTag>, E> {
                Ok(None)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Option<DefaultTag>, E> {
                Ok(None)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Option<DefaultTag>, E> {
                Ok(None)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Option<DefaultTag>, E> {
                Ok(None)
            }
            fn visit_none<E: de::Error>(self) -> Result<Option<DefaultTag>, E> {
                Ok(None)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<DefaultTag>, A::Error> {
                skip_seq(seq).map(|()| None)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Option<DefaultTag>, A::Error> {
                skip_map(map).map(|()| None)
            }
        }
        deserializer.deserialize_any(TagVisitor).map(OptDefaultTag)
    }
}

#[derive(Clone)]
enum SpecialNumTag {
    Nan,
    Inf,
    /// Any other text (a Faces', Axes' or Ray's default)
    Other(String),
}

struct OptSpecialNum(Option<SpecialNumTag>);

impl<'de> Deserialize<'de> for OptSpecialNum {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<OptSpecialNum, D::Error> {
        struct SVisitor;
        impl<'de> Visitor<'de> for SVisitor {
            type Value = Option<SpecialNumTag>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Option<SpecialNumTag>, E> {
                Ok(Some(match s {
                    "nan" => SpecialNumTag::Nan,
                    "inf" => SpecialNumTag::Inf,
                    other => SpecialNumTag::Other(other.to_string()),
                }))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Option<SpecialNumTag>, E> {
                Ok(None)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Option<SpecialNumTag>, E> {
                Ok(None)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Option<SpecialNumTag>, E> {
                Ok(None)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Option<SpecialNumTag>, E> {
                Ok(None)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Option<SpecialNumTag>, E> {
                Ok(None)
            }
            fn visit_none<E: de::Error>(self) -> Result<Option<SpecialNumTag>, E> {
                Ok(None)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<SpecialNumTag>, A::Error> {
                skip_seq(seq).map(|()| None)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Option<SpecialNumTag>, A::Error> {
                skip_map(map).map(|()| None)
            }
        }
        deserializer.deserialize_any(SVisitor).map(OptSpecialNum)
    }
}

enum DefaultKey {
    Tag,
    V,
    E,
    N,
    S,
    Other,
}

impl<'de> Deserialize<'de> for DefaultKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DefaultKey, D::Error> {
        struct DKVisitor;
        impl Visitor<'_> for DKVisitor {
            type Value = DefaultKey;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a key")
            }
            fn visit_str<E: de::Error>(self, key: &str) -> Result<DefaultKey, E> {
                Ok(match key {
                    "$t" => DefaultKey::Tag,
                    "v" => DefaultKey::V,
                    "e" => DefaultKey::E,
                    "n" => DefaultKey::N,
                    "s" => DefaultKey::S,
                    _ => DefaultKey::Other,
                })
            }
        }
        deserializer.deserialize_identifier(DKVisitor)
    }
}

fn default_v_nums(v: Option<DefaultV>) -> SmallNums {
    match v {
        Some(DefaultV::Array { nums, .. }) => nums,
        Some(DefaultV::Num(n)) => {
            let mut out = SmallNums::default();
            out.push(n);
            out
        }
        Some(DefaultV::Str(s)) => {
            let mut out = SmallNums::default();
            out.push(s.parse().unwrap_or(0.0));
            out
        }
        Some(DefaultV::Other) => {
            let mut out = SmallNums::default();
            out.push(0.0);
            out
        }
        None => SmallNums::default(),
    }
}

impl<'de> Deserialize<'de> for ParsedDefault {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ParsedDefault, D::Error> {
        struct DefaultVisitor;
        impl<'de> Visitor<'de> for DefaultVisitor {
            type Value = ParsedDefault;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<ParsedDefault, E> {
                Ok(ParsedDefault::Dom(Variant::Bool(b)))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<ParsedDefault, E> {
                Ok(ParsedDefault::Dom(Variant::Float64(n as f64)))
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<ParsedDefault, E> {
                Ok(ParsedDefault::Dom(Variant::Float64(n as f64)))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<ParsedDefault, E> {
                Ok(ParsedDefault::Dom(Variant::Float64(n)))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<ParsedDefault, E> {
                Ok(ParsedDefault::Dom(Variant::String(s.to_string())))
            }
            fn visit_string<E: de::Error>(self, s: String) -> Result<ParsedDefault, E> {
                Ok(ParsedDefault::Dom(Variant::String(s)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<ParsedDefault, E> {
                Ok(ParsedDefault::None)
            }
            fn visit_none<E: de::Error>(self) -> Result<ParsedDefault, E> {
                Ok(ParsedDefault::None)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<ParsedDefault, A::Error> {
                skip_seq(seq).map(|()| ParsedDefault::None)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ParsedDefault, A::Error> {
                let mut tag: Option<DefaultTag> = None;
                let mut v: Option<DefaultV> = None;
                let mut e: Option<String> = None;
                let mut n: Option<String> = None;
                let mut s: Option<SpecialNumTag> = None;
                while let Some(key) = map.next_key::<DefaultKey>()? {
                    match key {
                        DefaultKey::Tag => tag = map.next_value::<OptDefaultTag>()?.0,
                        DefaultKey::V => v = Some(map.next_value()?),
                        DefaultKey::E => e = map.next_value::<OptText>()?.0,
                        DefaultKey::N => n = map.next_value::<OptText>()?.0,
                        DefaultKey::S => s = map.next_value::<OptSpecialNum>()?.0,
                        DefaultKey::Other => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                let Some(tag) = tag else { return Ok(ParsedDefault::None) };
                Ok(match tag {
                    DefaultTag::Number => ParsedDefault::Dom(Variant::Float64(match s {
                        Some(SpecialNumTag::Nan) => f64::NAN,
                        Some(SpecialNumTag::Inf) => f64::INFINITY,
                        _ => f64::NEG_INFINITY,
                    })),
                    DefaultTag::EnumItem => match (e, n) {
                        (Some(e), Some(n)) => ParsedDefault::EnumItem(e, n),
                        _ => ParsedDefault::None,
                    },
                    DefaultTag::UDim => {
                        let a = default_v_nums(v);
                        ParsedDefault::Dom(Variant::UDim(dom::UDim::new(a.at(0) as f32, a.at(1) as i32)))
                    }
                    DefaultTag::UDim2 => {
                        let a = default_v_nums(v);
                        ParsedDefault::Dom(Variant::UDim2(dom::UDim2::new(
                            dom::UDim::new(a.at(0) as f32, a.at(1) as i32),
                            dom::UDim::new(a.at(2) as f32, a.at(3) as i32),
                        )))
                    }
                    DefaultTag::Vector2 => {
                        let a = default_v_nums(v);
                        ParsedDefault::Dom(Variant::Vector2(dom::Vector2::new(a.at(0) as f32, a.at(1) as f32)))
                    }
                    DefaultTag::Vector2int16 => {
                        let a = default_v_nums(v);
                        ParsedDefault::Dom(Variant::Vector2int16(dom::Vector2int16::new(a.at(0) as i16, a.at(1) as i16)))
                    }
                    DefaultTag::Vector3 => {
                        let a = default_v_nums(v);
                        ParsedDefault::Dom(Variant::Vector3(dom::Vector3::new(a.at(0) as f32, a.at(1) as f32, a.at(2) as f32)))
                    }
                    DefaultTag::Vector3int16 => {
                        let a = default_v_nums(v);
                        ParsedDefault::Dom(Variant::Vector3int16(dom::Vector3int16::new(
                            a.at(0) as i16,
                            a.at(1) as i16,
                            a.at(2) as i16,
                        )))
                    }
                    DefaultTag::Color3 => {
                        let a = default_v_nums(v);
                        ParsedDefault::Dom(Variant::Color3(dom::Color3::new(a.at(0) as f32, a.at(1) as f32, a.at(2) as f32)))
                    }
                    DefaultTag::BrickColor => {
                        let a = default_v_nums(v);
                        let color = dom::BrickColor::from_number(a.at(0) as u16).unwrap_or(dom::BrickColor::MediumStoneGrey);
                        ParsedDefault::Dom(Variant::BrickColor(color))
                    }
                    DefaultTag::CFrame => {
                        let a = default_v_nums(v);
                        let f = |i| a.at(i) as f32;
                        let cframe = if a.len >= 12 {
                            dom::CFrame::new(
                                dom::Vector3::new(f(0), f(1), f(2)),
                                dom::Matrix3::new(
                                    dom::Vector3::new(f(3), f(4), f(5)),
                                    dom::Vector3::new(f(6), f(7), f(8)),
                                    dom::Vector3::new(f(9), f(10), f(11)),
                                ),
                            )
                        } else {
                            dom::CFrame::new(dom::Vector3::new(f(0), f(1), f(2)), dom::Matrix3::identity())
                        };
                        ParsedDefault::Dom(Variant::CFrame(cframe))
                    }
                    DefaultTag::Rect => {
                        let a = default_v_nums(v);
                        ParsedDefault::Dom(Variant::Rect(dom::Rect::new(
                            dom::Vector2::new(a.at(0) as f32, a.at(1) as f32),
                            dom::Vector2::new(a.at(2) as f32, a.at(3) as f32),
                        )))
                    }
                    DefaultTag::NumberRange => {
                        let a = default_v_nums(v);
                        ParsedDefault::Dom(Variant::NumberRange(dom::NumberRange::new(a.at(0) as f32, a.at(1) as f32)))
                    }
                    DefaultTag::NumberSequence => {
                        let keypoints = match v {
                            Some(DefaultV::Array { nested: Some(rows), .. }) => rows
                                .into_iter()
                                .map(|k| dom::NumberSequenceKeypoint::new(k[0] as f32, k[1] as f32, k[2] as f32))
                                .collect(),
                            Some(DefaultV::Array { nums, .. }) => (0..nums.len)
                                .map(|i| dom::NumberSequenceKeypoint::new(nums.at(i) as f32, 0.0, 0.0))
                                .collect(),
                            _ => Vec::new(),
                        };
                        ParsedDefault::Dom(Variant::NumberSequence(dom::NumberSequence { keypoints }))
                    }
                    DefaultTag::ColorSequence => {
                        let keypoints = match v {
                            Some(DefaultV::Array { nested: Some(rows), .. }) => rows
                                .into_iter()
                                .map(|k| {
                                    dom::ColorSequenceKeypoint::new(
                                        k[0] as f32,
                                        dom::Color3::new(k[1] as f32, k[2] as f32, k[3] as f32),
                                    )
                                })
                                .collect(),
                            Some(DefaultV::Array { nums, .. }) => (0..nums.len)
                                .map(|i| dom::ColorSequenceKeypoint::new(nums.at(i) as f32, dom::Color3::new(0.0, 0.0, 0.0)))
                                .collect(),
                            _ => Vec::new(),
                        };
                        ParsedDefault::Dom(Variant::ColorSequence(dom::ColorSequence { keypoints }))
                    }
                    DefaultTag::Font => match v {
                        Some(DefaultV::Array { strings, .. }) => {
                            let mut parts = strings.unwrap_or_default().into_iter();
                            let family = parts.next().flatten().unwrap_or_default();
                            let weight = parts.next().flatten().unwrap_or_default();
                            let style = parts.next().flatten().unwrap_or_default();
                            ParsedDefault::Font(family, weight, style)
                        }
                        _ => ParsedDefault::None,
                    },
                    DefaultTag::Content => match v {
                        Some(DefaultV::Str(uri)) if !uri.is_empty() => ParsedDefault::Dom(Variant::Content(dom::Content::from_uri(uri))),
                        _ => ParsedDefault::Dom(Variant::Content(dom::Content::none())),
                    },
                    DefaultTag::Faces | DefaultTag::Axes | DefaultTag::Ray => match s {
                        Some(SpecialNumTag::Other(text)) => {
                            let kind = match tag {
                                DefaultTag::Faces => "Faces",
                                DefaultTag::Axes => "Axes",
                                _ => "Ray",
                            };
                            crate::value::text_default(kind, &text).map_or(ParsedDefault::None, ParsedDefault::Dom)
                        }
                        _ => ParsedDefault::None,
                    },
                    DefaultTag::Other => ParsedDefault::None,
                })
            }
        }
        deserializer.deserialize_any(DefaultVisitor)
    }
}

struct ParsedDefaults(Option<Vec<(String, ParsedDefault)>>);

impl<'de> Deserialize<'de> for ParsedDefaults {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ParsedDefaults, D::Error> {
        struct DefaultsVisitor;
        impl<'de> Visitor<'de> for DefaultsVisitor {
            type Value = Option<Vec<(String, ParsedDefault)>>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(Option<Vec<(String, ParsedDefault)>>);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<Vec<(String, ParsedDefault)>>, A::Error> {
                skip_seq(seq).map(|()| Some(Vec::new()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<Vec<(String, ParsedDefault)>>, A::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, ParsedDefault>()? {
                    entries.push((key, value));
                }
                Ok(Some(entries))
            }
        }
        deserializer.deserialize_any(DefaultsVisitor).map(ParsedDefaults)
    }
}

/// A member list entry's `name`: the string value of an object's last `name` key, or `None` (an entry that is no object,
/// or whose name is no string).
struct EntryName(Option<String>);

impl<'de> Deserialize<'de> for EntryName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<EntryName, D::Error> {
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
                        Key::Name => name = map.next_value::<OptText>()?.0,
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
                        Key::Name => class.name = map.next_value::<OptText>()?.0,
                        Key::Superclass => class.superclass = map.next_value::<OptText>()?.0,
                        Key::Serialized => class.serialized = map.next_value::<IsTrue>()?.0,
                        Key::Permits => class.permits = map.next_value()?,
                        Key::Display => class.display = map.next_value::<ParsedDisplay>()?.0,
                        Key::Properties => class.properties = map.next_value::<PropertiesList>()?.0,
                        Key::Defaults => class.defaults = map.next_value::<ParsedDefaults>()?.0,
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

/// A JSON array's items read as `T`, or none for any other value.
struct List<T>(Vec<T>);

impl<T> Default for List<T> {
    fn default() -> Self {
        List(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for List<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<List<T>, D::Error> {
        struct ListVisitor<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for ListVisitor<T> {
            type Value = List<T>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(List<T>);
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<List<T>, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element::<T>()? {
                    items.push(item);
                }
                Ok(List(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<List<T>, A::Error> {
                skip_map(map).map(|()| List::default())
            }
        }
        deserializer.deserialize_any(ListVisitor(PhantomData))
    }
}

impl<'de> Deserialize<'de> for MemberClass {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<MemberClass, D::Error> {
        struct ClassVisitor;
        impl<'de> Visitor<'de> for ClassVisitor {
            type Value = MemberClass;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(MemberClass);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<MemberClass, A::Error> {
                skip_seq(seq).map(|()| MemberClass::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<MemberClass, A::Error> {
                let mut class = MemberClass::default();
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Name => class.name = Some(map.next_value()?),
                        Key::Events => class.events = Some(map.next_value()?),
                        Key::Methods => class.methods = Some(map.next_value()?),
                        Key::Callbacks => class.callbacks = Some(map.next_value()?),
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

impl<'de> Deserialize<'de> for DumpLoad {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DumpLoad, D::Error> {
        struct DumpVisitor;
        impl<'de> Visitor<'de> for DumpVisitor {
            type Value = DumpLoad;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(DumpLoad);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<DumpLoad, A::Error> {
                skip_seq(seq).map(|()| DumpLoad::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<DumpLoad, A::Error> {
                let mut dump = DumpLoad::default();
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Version => dump.version = map.next_value::<OptText>()?.0,
                        Key::Enums => dump.enums = map.next_value::<EnumsList>()?.0,
                        Key::Classes => dump.classes = map.next_value::<List<DumpClass>>()?.0,
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

impl<'de> Deserialize<'de> for MemberDump {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<MemberDump, D::Error> {
        struct MemberDumpVisitor;
        impl<'de> Visitor<'de> for MemberDumpVisitor {
            type Value = MemberDump;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            scalars_read_empty!(MemberDump);
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<MemberDump, A::Error> {
                skip_seq(seq).map(|()| MemberDump::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<MemberDump, A::Error> {
                let mut dump = MemberDump::default();
                while let Some(key) = map.next_key::<Key>()? {
                    match key {
                        Key::Classes => dump.classes = map.next_value::<List<MemberClass>>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(dump)
            }
        }
        deserializer.deserialize_any(MemberDumpVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflection::{BEHAVIOR_ONLY_NAMES, K};

    /// The members `class` has only through the engine's behaviours, by the lists ([`BEHAVIOR_METHODS`],
    /// [`BEHAVIOR_PROPERTIES`]): (kind, name), for the class and its subclasses.
    fn behaviour_only(reflection: &Reflection, class: &ClassInfo) -> Vec<(MemberKind, NameId)> {
        let methods = BEHAVIOR_METHODS.iter().flat_map(|(owner, names)| names.iter().map(move |name| (*owner, MemberKind::Method, *name)));
        let properties = BEHAVIOR_PROPERTIES.iter().map(|&(owner, name, ..)| (owner, MemberKind::Property, name));
        methods
            .chain(properties)
            .filter(|&(owner, ..)| reflection.is_a(class.id, owner))
            .map(|(_, kind, name)| (kind, reflection.name_id(name).unwrap()))
            .filter(|&(kind, name)| if kind == MemberKind::Method { !class.methods.contains(&name) } else { !class.properties.contains_key(&name) })
            .collect()
    }

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
        // every class's list is the members the engine resolves for it, once each: the reflection's and the ones it has only
        // through behaviours
        for class in reflection.all_classes() {
            let behaviour = behaviour_only(&reflection, class);
            let mut seen = FxHashMap::default();
            for member in reflection.members(class.id) {
                let known = match member {
                    ReflectedMember::Property(property) => class.properties.get(&property.name).map(|&index| reflection.property(index)).is_some_and(|found| ptr::eq(found, property)),
                    ReflectedMember::Method(method) => class.methods.contains(&method.name),
                    ReflectedMember::Event(event) => class.events.contains(&event.name),
                    ReflectedMember::Callback(callback) => class.callbacks.contains(&callback.name),
                } || (member.origin() == MemberOrigin::Engine && behaviour.contains(&(member.kind(), member.name())));
                assert!(known, "{}.{}", class.name, reflection.name(member.name()));
                assert!(seen.insert((member.kind(), member.name()), ()).is_none(), "{}.{} twice", class.name, reflection.name(member.name()));
            }
            let reflected = class.properties.len() + class.methods.len() + class.events.len() + class.callbacks.len();
            assert_eq!(seen.len(), reflected + behaviour.len(), "{}", class.name);
        }
    }

    /// The members the metadata lists as the engine's behaviours are the ones a started engine has behaviours for that the
    /// reflection lacks, in the client, server and edit run contexts, a property read-only where it has no setter.
    #[test]
    fn behaviour_only_members_are_the_engines_behaviours() {
        let mut listed: Vec<(String, &str, String, bool)> = BEHAVIOR_METHODS
            .iter()
            .flat_map(|(class, names)| names.iter().map(move |name| (class.to_string(), "method", name.to_string(), false)))
            .chain(BEHAVIOR_PROPERTIES.iter().map(|&(class, name, _, _, read_only, _)| (class.to_string(), "property", name.to_string(), read_only)))
            .collect();
        crate::sort::ord(&mut listed);
        assert_eq!(listed.iter().filter(|entry| entry.1 == "method").count(), BEHAVIOR_ONLY_NAMES.len());
        for (mode, context) in [("play", crate::RunContext::Client), ("play", crate::RunContext::Server), ("edit", crate::RunContext::Edit)] {
            let engine = crate::Engine::new(crate::Options { mode: mode.into(), run_context: context, ..crate::Options::default() }).unwrap();
            let lua = mlua::Lua::new();
            crate::lua::start(&lua, &engine).unwrap();
            let reflection = &engine.reflection;
            let behaviors = engine.behaviors.borrow();
            let mut found = Vec::new();
            for (class, name) in behaviors.method_keys() {
                let info = reflection.class(class);
                if !info.methods.contains(&name) {
                    found.push((info.name.to_string(), "method", reflection.name(name).to_string(), false));
                }
            }
            for (class, name) in behaviors.getter_keys() {
                let info = reflection.class(class);
                if !info.properties.contains_key(&name) {
                    let read_only = behaviors.find_setter(reflection, class, name).is_none();
                    found.push((info.name.to_string(), "property", reflection.name(name).to_string(), read_only));
                }
            }
            crate::sort::ord(&mut found);
            assert_eq!(found, listed, "{mode} {}", context.name());
        }
        // the metadata finds them, as the engine's members with no recorded signature
        let reflection = Reflection::shared().unwrap();
        let Some(ReflectedMember::Method(kick)) = reflection.member_by_name("Player", "Kick") else { panic!("Player.Kick") };
        assert_eq!((kick.origin, kick.parameters.len(), kick.returns.is_none(), reflection.class(kick.owner).name.as_ref()), (MemberOrigin::Engine, 0, true, "Player"));
        let player = reflection.class_id("Player").unwrap();
        let own: Vec<&str> = reflection.own_methods(player).iter().map(|method| reflection.name(method.name)).collect();
        assert!(BEHAVIOR_ONLY_NAMES.iter().all(|name| own.contains(name)), "{own:?}");
        assert!(reflection.members(player).any(|member| reflection.name(member.name()) == "LoadCharacter"));
        let Some(ReflectedMember::Property(source)) = reflection.member_by_name("ModuleScript", "Source") else { panic!("ModuleScript.Source") };
        assert_eq!((source.origin, &*source.script_type_name, source.read_only, source.scriptable), (MemberOrigin::Engine, "string", false, true));
        assert_eq!(reflection.class(source.owner).name.as_ref(), "LuaSourceContainer");
        // (files hold a script's Source: rbx-dom's database serializes it)
        let dom = ["LuaSourceContainer", "BaseScript", "Script", "ModuleScript"]
            .into_iter()
            .flat_map(|class| reflection.dom_properties(reflection.class_id(class).unwrap()))
            .find(|property| property.name == "Source")
            .unwrap();
        assert_eq!((source.serialized, dom.serialization), (true, Serialization::Serializes));
        let module = reflection.class_id("ModuleScript").unwrap();
        assert!(reflection.members(module).any(|member| member == ReflectedMember::Property(source)));
        let Some(ReflectedMember::Property(localized)) = reflection.member_by_name("TextBox", "LocalizedText") else { panic!("TextBox.LocalizedText") };
        assert_eq!((localized.origin, localized.read_only), (MemberOrigin::Engine, true));
        let Some(ReflectedMember::Property(label)) = reflection.member_by_name("TextLabel", "LocalizedText") else { panic!("TextLabel.LocalizedText") };
        assert_eq!((label.origin, label.category()), (MemberOrigin::Dump, Some("Data")));
        // (the engine's own lookups are as they were: no reflected member of these names)
        assert!(reflection.class_property(module, K::Source as NameId).is_none() && !reflection.class(player).methods.contains(&kick.name));
    }

    #[test]
    fn enums_list_their_items_and_legacy_names() {
        let reflection = Reflection::shared().unwrap();
        let font = reflection.enum_by_name("Font").unwrap();
        let medium = reflection.enum_item("Font", "GothamMedium").unwrap();
        let mut legacy: Vec<&str> = reflection.legacy_names(medium).collect();
        crate::sort::ord(&mut legacy);
        assert_eq!(legacy, ["GothamSemibold", "MontserratMedium"]);
        assert!(reflection.enum_items_of(font.id).all(|item| reflection.legacy_names(item.id).all(|name| name != &*item.name)));
        assert!(reflection.enum_items_of(font.id).any(|item| item.id == medium));
        assert_eq!(reflection.all_enums().count(), 662);
        for (index, item) in reflection.enum_items.iter().enumerate() {
            assert_eq!(item.id as usize, index);
            for legacy in reflection.legacy_names(item.id) {
                assert_eq!(reflection.enum_item(reflection.enum_item_type_name(item.id), legacy), Some(item.id), "{legacy}");
            }
        }
        // a legacy name an item of the enum already has is not installed, and is not that item's legacy name
        let dump = br#"{"classes": [], "enums": [{"name": "KeyCode", "items": [{"name": "None", "value": 0}, {"name": "Unknown", "value": 1}]}]}"#;
        let own = Reflection::from_dump(dump, None).unwrap();
        let none = own.enum_item("KeyCode", "None").unwrap();
        assert_eq!(own.legacy_names(none).collect::<Vec<_>>(), Vec::<&str>::new());
        assert_ne!(own.enum_item("KeyCode", "Unknown"), Some(none));
        let none = reflection.enum_item("KeyCode", "None").unwrap();
        assert!(reflection.legacy_names(none).any(|name| name == "Unknown"));
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
        // (Debug gives the dump the info is built from as its size)
        assert_eq!(format!("{:?}", reflection.member_source), format!("DumpSource({} bytes)", crate::data::REFLECTION.len()));
        // from_dump builds it at once, the same
        let read = Reflection::from_dump(crate::data::REFLECTION, None).unwrap();
        assert!(read.members.get().is_some());
        for class in reflection.all_classes() {
            assert_eq!(read.own_methods(class.id), reflection.own_methods(class.id));
            assert_eq!(read.own_events(class.id), reflection.own_events(class.id));
            assert_eq!(read.own_callbacks(class.id), reflection.own_callbacks(class.id));
        }
    }

    /// A class entry as a test compares the readers: its name, its events', methods' and callbacks' names, and the rest.
    type ClassRead = (Option<String>, Vec<String>, Vec<String>, Vec<String>, String);

    /// What the load read from a dump when it held the whole file as JSON values: each class's name, member lists,
    /// properties, permits, display, and defaults decoded through `decode_json`.
    fn read_as_values(bytes: &[u8]) -> (Option<String>, Vec<(String, Vec<(String, u64)>)>, Vec<ClassRead>) {
        let dump: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let version = text(dump.get("version")).map(str::to_string);
        let enums: Vec<(String, Vec<(String, u64)>)> = list(dump.get("enums"))
            .iter()
            .filter_map(|entry| {
                let name = text(entry.get("name"))?.to_string();
                let items = list(entry.get("items"))
                    .iter()
                    .filter_map(|item| Some((text(item.get("name"))?.to_string(), item.get("value")?.as_f64()?.to_bits())))
                    .collect();
                Some((name, items))
            })
            .collect();
        let classes = list(dump.get("classes"))
            .iter()
            .map(|entry| {
                let names = |key: &str| list(entry.get(key)).iter().filter_map(|e| text(e.get("name"))).map(str::to_string).collect::<Vec<_>>();
                let permits = entry.get("permits");
                let has_key = |v: Option<&serde_json::Value>, k: &str| v.and_then(|v| v.as_object()).is_some_and(|m| m.contains_key(k));
                let props: Vec<String> = list(entry.get("properties"))
                    .iter()
                    .filter_map(|p| {
                        let name = text(p.get("name"))?;
                        let k = p.get("type");
                        let pp = p.get("permits");
                        Some(format!(
                            "{name}|{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{:?}|{:?}",
                            text(k.and_then(|k| k.get("script"))),
                            text(k.and_then(|k| k.get("engine"))),
                            text(k.and_then(|k| k.get("enum"))),
                            text(k.and_then(|k| k.get("instance"))),
                            !has_key(pp, "Write"),
                            has_key(pp, "Read"),
                            p.get("serialized").and_then(|v| v.as_bool()) == Some(true),
                            display_from_json(p.get("display")),
                            Permits::from_json(pp),
                        ))
                    })
                    .collect();
                let has_defaults = matches!(entry.get("defaults"), Some(serde_json::Value::Object(_) | serde_json::Value::Array(_)));
                let rest = format!(
                    "{:?};{};{};{};{:?};{:?};{};{:?}",
                    text(entry.get("superclass")),
                    entry.get("serialized").and_then(|v| v.as_bool()) == Some(true),
                    has_key(permits, "New"),
                    has_key(permits, "GetService"),
                    Permits::from_json(permits),
                    display_from_json(entry.get("display")),
                    has_defaults,
                    props,
                );
                (text(entry.get("name")).map(str::to_string), names("events"), names("methods"), names("callbacks"), rest)
            })
            .collect();
        (version, enums, classes)
    }

    fn read_as_dump(bytes: &[u8]) -> (Option<String>, Vec<(String, Vec<(String, u64)>)>, Vec<ClassRead>) {
        let dump: DumpLoad = serde_json::from_slice(bytes).unwrap();
        let enums = dump
            .enums
            .into_iter()
            .map(|e| (e.name, e.items.into_iter().map(|i| (i.name, i.value.to_bits())).collect()))
            .collect();
        let classes = dump
            .classes
            .into_iter()
            .map(|class| {
                let props: Vec<String> = class
                    .properties
                    .into_iter()
                    .map(|p| {
                        format!(
                            "{}|{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{:?}|{:?}",
                            p.name,
                            p.script.as_deref(),
                            p.engine.as_deref(),
                            p.enum_name.as_deref(),
                            p.instance_type.as_deref(),
                            p.read_only,
                            p.scriptable,
                            p.serialized,
                            p.display,
                            p.permits,
                        )
                    })
                    .collect();
                let rest = format!(
                    "{:?};{};{};{};{:?};{:?};{};{:?}",
                    class.superclass.as_deref(),
                    class.serialized,
                    class.permits.has_new,
                    class.permits.has_get_service,
                    class.permits.permits,
                    class.display,
                    class.defaults.is_some(),
                    props,
                );
                (class.name, class.events, class.methods, class.callbacks, rest)
            })
            .collect();
        (dump.version, enums, classes)
    }

    #[test]
    fn the_load_reads_what_the_json_values_held() {
        assert_eq!(read_as_dump(crate::data::REFLECTION), read_as_values(crate::data::REFLECTION));
        // Every decoded default of every class in the compiled-in dump matches `value::decode_json` on the raw JSON values.
        let reflection = Reflection::shared().unwrap();
        let raw_dump: serde_json::Value = serde_json::from_slice(crate::data::REFLECTION).unwrap();
        for entry in list(raw_dump.get("classes")) {
            let Some(class_name) = text(entry.get("name")) else { continue };
            let class = reflection.class_by_name(class_name).unwrap();
            if let Some(serde_json::Value::Object(defaults)) = entry.get("defaults") {
                let loaded = class.raw_defaults.as_ref().unwrap();
                for (prop_name, json_val) in defaults {
                    let expected = crate::value::decode_json(&reflection, json_val);
                    let got = reflection
                        .name_id(prop_name)
                        .and_then(|id| loaded.get(&id))
                        .and_then(|raw| crate::value::decode_default(&reflection, raw));
                    match (&expected, &got) {
                        (Some(Value::Number(a)), Some(Value::Number(b))) if a.is_nan() && b.is_nan() => {}
                        _ => assert_eq!(format!("{got:?}"), format!("{expected:?}"), "{class_name}.{prop_name}"),
                    }
                }
            }
        }
        // entries of other shapes: not an object, lists that are `{}` or no list, names that are no string, repeated and
        // escaped keys, escaped names, nesting
        let odd = br#"{"version": "x", "classes": [
            5, [1, {"name": "A"}], null, "Part",
            {"name": "A", "events": {}, "methods": [{"name": "M", "parameters": [{"name": "p", "default": {"$t": "table", "v": {"name": "no"}}}]}, {"name": 3}, 7, [], {"na\u006de": "Esc\u0061ped"}, {"other": {"name": "deep"}}], "callbacks": "none"},
            {"name": 5, "methods": [{"name": "Lost"}]},
            {"name": "B", "name": "C", "methods": [{"name": "X", "name": "Y"}, {"name": "Z", "name": null}], "methods": [{"name": "Last"}], "events": [{"name": "E"}], "superclass": "A", "defaults": {"Size": {"$t": "Vector3", "v": [1, 2, 3]}}}
        ], "enums": []}"#;
        assert_eq!(read_as_dump(odd), read_as_values(odd));
        let (_, _, classes) = read_as_dump(odd);
        assert_eq!(classes[4].2, ["M", "Escaped"]);
        // the member info reads the same entries, by the same rules (from_dump builds it at once)
        let reflection = Reflection::from_dump(odd, None).unwrap();
        let methods = |class: &str| -> Vec<(String, MemberOrigin)> {
            reflection.own_methods(reflection.class_id(class).unwrap()).iter().map(|method| (reflection.name(method.name).to_string(), method.origin)).collect()
        };
        assert_eq!(methods("A"), [("M".to_string(), MemberOrigin::Dump), ("Escaped".to_string(), MemberOrigin::Dump)]);
        assert_eq!(&*reflection.own_methods(reflection.class_id("A").unwrap())[0].parameters[0].name, "p");
        assert_eq!(methods("C"), [("Last".to_string(), MemberOrigin::Dump)]);
        assert_eq!(reflection.own_events(reflection.class_id("C").unwrap())[0].origin, MemberOrigin::Dump);
        for top in [&br#"[]"#[..], br#"5"#, br#"{"classes": {}}"#, br#"{"classes": 3}"#, br#"{}"#] {
            assert_eq!(read_as_dump(top), read_as_values(top));
        }
    }

    #[test]
    fn undecodable_and_long_sequence_defaults_match_raw_json_decode() {
        let dump = br#"{
            "version": "x",
            "enums": [],
            "classes": [
                {
                    "name": "CustomBase",
                    "defaults": {
                        "Name": null,
                        "UnknownEnum": { "$t": "EnumItem", "e": "NoSuchEnum", "n": "None" },
                        "UnknownFont": { "$t": "Font", "v": ["rbxasset://fonts/families/Arial.json", "NoSuchWeight", "Normal"] },
                        "BadTag": { "$t": "NotAType", "v": 1 },
                        "LongNums": { "$t": "NumberSequence", "v": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14] },
                        "LongColors": { "$t": "ColorSequence", "v": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14] }
                    }
                },
                {
                    "name": "CustomSub",
                    "superclass": "CustomBase",
                    "defaults": {}
                }
            ]
        }"#;
        let reflection = Reflection::from_dump(dump, None).unwrap();
        let base_id = reflection.class_id("CustomBase").unwrap();
        let sub_id = reflection.class_id("CustomSub").unwrap();
        let base_defaults = reflection.class(base_id).raw_defaults.as_ref().unwrap();
        for key in ["Name", "UnknownEnum", "UnknownFont", "BadTag"] {
            let id = reflection.name_id(key).unwrap();
            assert!(base_defaults.contains_key(&id), "missing raw default key {key}");
            assert!(crate::value::decode_default(&reflection, &base_defaults[&id]).is_none());
        }
        // Superclass carries key `Name` (even though undecodable): subclass delegates to superclass's `compute_default`.
        let name = crate::dom::compute_default(&reflection, sub_id, crate::reflection::K::Name as NameId);
        assert_eq!(name.as_ref().and_then(Value::as_str).map(|s| s.to_str_lossy().into_owned()).as_deref(), Some("CustomBase"));
        // Sequences with more than 12 bare numbers keep every keypoint and match `decode_json`.
        let raw_json: serde_json::Value = serde_json::from_slice(dump).unwrap();
        let raw_base_defaults = raw_json["classes"][0]["defaults"].as_object().unwrap();
        for key in ["LongNums", "LongColors"] {
            let id = reflection.name_id(key).unwrap();
            let got = crate::value::decode_default(&reflection, &base_defaults[&id]).unwrap();
            let expected = crate::value::decode_json(&reflection, &raw_base_defaults[key]).unwrap();
            assert_eq!(format!("{got:?}"), format!("{expected:?}"), "{key}");
            match &got {
                Value::NumberSequence(seq) => {
                    assert_eq!(seq.keypoints.len(), 15);
                    assert_eq!(seq.keypoints[14].time, 14.0);
                }
                Value::ColorSequence(seq) => {
                    assert_eq!(seq.keypoints.len(), 15);
                    assert_eq!(seq.keypoints[14].time, 14.0);
                }
                other => panic!("unexpected {key}: {other:?}"),
            }
        }
    }
}
