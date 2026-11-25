//! The instance arena: generational ids, class, name, parent, ordered children, properties by
//! interned name, attributes, tags, flags and change versions. Pure data and tree queries; the
//! script-visible semantics (signals, hooks, errors) live in `engine::instance`.

use std::rc::Rc;

use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    reflection::{ClassId, K, NameId, Reflection},
    signal::SignalId,
    value::{Str, Value},
};

/// A generational instance handle. `index` addresses the arena slot; `generation` guards a
/// reused slot: `lua::raw::collect` frees unreachable parentless instances (their slot's
/// generation is bumped and the slot reused), so `Dom::get` of a stale id is `None`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct InstanceId {
    pub index: u32,
    pub generation: u32,
}

pub mod flags {
    pub const DESTROYED: u16 = 1;
    pub const PARENT_LOCKED: u16 = 1 << 1;
    /// Made by a script (Instance.new): never stands for a Studio-only instance of the place
    pub const SCRIPT_CREATED: u16 = 1 << 2;
    /// A Studio-only placeholder from the place's hierarchy snapshot
    pub const STUDIO_ONLY: u16 = 1 << 3;
    /// Studio-only children were added (Hooks.Materialize ran)
    pub const MATERIALIZED: u16 = 1 << 4;
    /// A Rojo JSON module: `require` returns the decoded file
    pub const JSON_MODULE: u16 = 1 << 5;
    /// An instance reference was stored among its properties at some point (set by every write
    /// path, never cleared): the instance collector looks for references only in these
    pub const HOLDS_REFS: u16 = 1 << 6;
    /// Made from a model plan imported lazily (project::LazyImport): its subtree is made as it is
    /// looked at
    pub const LAZY_REGION: u16 = 1 << 7;
    /// Its plan children are not made yet (they are made before anything lists or finds them)
    pub const LAZY_PENDING: u16 = 1 << 8;
    /// A lazily imported plan's roots are below it (a bundle folder or one of its ancestors)
    pub const LAZY_ABOVE: u16 = 1 << 9;
    /// Stands where a manifest bundle's root goes and is not that root yet: it becomes it (class,
    /// properties, attributes, tags) before a script finds, tests, touches or looks inside it
    /// (project::model_assets::reveal)
    pub const BUNDLE_ROOT: u16 = 1 << 10;
    /// A loopback run's server instance inside the replicated containers (src/loopback/replicate.rs): its changes go to the
    /// client
    pub const REPLICATED: u16 = 1 << 11;
    /// The loopback client has (or is sent) a copy of it
    pub const SENT: u16 = 1 << 12;
    /// It left the replicated containers after it was sent: what it holds is sent again when it comes back
    pub const STALE: u16 = 1 << 13;
    /// The spatial index holds it (world/broadphase.rs): its writes go into [`super::Watch`]'s log
    pub const INDEXED: u16 = 1 << 14;
    /// In [`super::Watch::written`] until the index takes the log
    pub const LOGGED: u16 = 1 << 15;
}

/// Signals created for an instance (events by name, property / attribute change signals).
#[derive(Default, Debug)]
pub struct InstanceSignals {
    pub events: Vec<(NameId, SignalId)>,
    pub properties: Vec<(NameId, SignalId)>,
    pub attributes: Vec<(Str, SignalId)>,
}

impl InstanceSignals {
    pub fn event(&self, name: NameId) -> Option<SignalId> {
        self.events.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
    }

    pub fn property(&self, name: NameId) -> Option<SignalId> {
        self.properties.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
    }

    pub fn attribute(&self, name: &[u8]) -> Option<SignalId> {
        self.attributes.iter().find(|(n, _)| n.as_bytes() == name).map(|(_, s)| *s)
    }

    pub fn all(&self) -> impl Iterator<Item = SignalId> + '_ {
        self.events
            .iter()
            .map(|(_, s)| *s)
            .chain(self.properties.iter().map(|(_, s)| *s))
            .chain(self.attributes.iter().map(|(_, s)| *s))
    }
}

/// Where a mounted instance sits in the place's Studio hierarchy snapshot (project.luau
/// StudioNodeOf: cached per instance).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StudioNode {
    #[default]
    Unknown,
    None,
    Node(u32),
}

#[derive(Debug)]
pub struct Instance {
    pub class: ClassId,
    /// Creation serial (the Luau runtime's State.Id): the exported node id
    pub serial: u32,
    /// Effective name (RawGet(Name)): the stored Name, else the class's recorded default
    pub name: Str,
    pub parent: Option<InstanceId>,
    pub children: Children,
    /// Its place in its parent's [`Children`]
    place: u32,
    /// Explicitly stored properties, sorted by name; `Value::Nil` is a stored nil
    pub props: Props,
    /// Attributes in insertion order (AttributeOrder)
    pub attributes: Attributes,
    /// Tags in the order they were added; a tag given other than through `instance::add_tag` is
    /// recorded with `services::note_tag`
    pub tags: ThinVec<Str>,
    pub flags: u16,
    /// Where a script's or module's source comes from (none for every other instance: boxed, so an
    /// instance holds one pointer for it, not 48 bytes)
    pub script: Option<Box<ScriptSource>>,
    pub studio_node: StudioNode,
    pub signals: Option<Box<InstanceSignals>>,
    /// Callback members assigned by scripts (OnInvoke, OnClientInvoke, ...)
    pub callbacks: ThinVec<(NameId, mlua::Value)>,
    /// Properties a model file held that the engine does not model (a MeshPart's InitialSize and
    /// physics data, ...), kept as read so the instance can be written back out (engine::record)
    pub unmodeled: Option<crate::project::Unmodeled>,
    /// Bumped on every own property / attribute / tag change
    pub version: u32,
    /// Bumped when the child list changes
    pub child_version: u32,
}

/// An instance's children in their order, the order GetChildren lists them in, which is the order a playing client keeps them in
/// (Studio 0.747, a client and a server alike, every container: tools/parity/probes/child_order.luau). A client appends a child
/// it is given and takes one out as [`Children::take`] does: a last child is dropped, a list of [`CLIENT_KEEPS_ORDER`] or fewer
/// closes the gap, and a longer one moves its last child into the taken one's place. So once a child goes from the middle of a
/// long list its last child lists in its place (GetChildren, GetDescendants, FindFirstChild among children of one name,
/// ClearAllChildren's and Destroy's order, a Clone's), and taking out a child costs the same host time wherever it is and
/// however many children there are, as each child knows its place ([`Instance::place`]). (The dom once looked through the list
/// for the child and closed the gap: moving a million folders out of one, one by one, took 118 s.)
#[derive(Debug, Default)]
pub struct Children {
    slots: Vec<InstanceId>,
}

/// The longest child list a client closes the gap in when it takes a child out: of 20 the order stays, of 21 the last child takes
/// the place ([`Children`])
const CLIENT_KEEPS_ORDER: usize = 20;

impl Children {
    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The children in order.
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, InstanceId> {
        self.slots.iter()
    }

    #[inline]
    pub fn first(&self) -> Option<&InstanceId> {
        self.slots.first()
    }

    #[inline]
    pub fn last(&self) -> Option<&InstanceId> {
        self.slots.last()
    }

    pub fn to_vec(&self) -> Vec<InstanceId> {
        self.slots.clone()
    }

    /// The children in order, as a slice.
    #[inline]
    pub fn slice(&self) -> &[InstanceId] {
        &self.slots
    }

    /// Appends the children to `out` (a walk's stack), in order.
    #[inline]
    pub fn copy_to(&self, out: &mut Vec<InstanceId>) {
        out.extend_from_slice(&self.slots);
    }

    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    /// Takes out the child at `place` as a client does: the last dropped, a gap closed in a list of [`CLIENT_KEEPS_ORDER`] or fewer,
    /// else the last child moved into the place. The children whose places changed are those from `place` on: all of them for a
    /// closed gap, the one moved in otherwise.
    fn take(&mut self, place: usize) -> std::ops::Range<usize> {
        let len = self.slots.len();
        if place + 1 == len {
            self.slots.pop();
            place..place
        } else if len <= CLIENT_KEEPS_ORDER {
            self.slots.remove(place);
            place..len - 1
        } else {
            self.slots.swap_remove(place);
            place..place + 1
        }
    }
}

impl<'a> IntoIterator for &'a Children {
    type Item = &'a InstanceId;
    type IntoIter = std::slice::Iter<'a, InstanceId>;

    #[inline]
    fn into_iter(self) -> std::slice::Iter<'a, InstanceId> {
        self.slots.iter()
    }
}

/// A script's source, the file it is read from and the stand-in package it comes from ([`Instance::script`]).
#[derive(Clone, Debug, Default)]
pub struct ScriptSource {
    /// Script source (ModuleScript / Script) when held in memory
    pub source: Option<Rc<[u8]>>,
    /// File the source is read from (project scripts, JSON modules)
    pub source_path: Option<Rc<str>>,
    /// Offline stand-in package file this module's source comes from
    pub stand_in: Option<Rc<str>>,
}

impl Instance {
    /// Its source when held in memory.
    #[inline]
    pub fn source(&self) -> Option<&Rc<[u8]>> {
        self.script.as_ref()?.source.as_ref()
    }

    /// The file its source is read from.
    #[inline]
    pub fn source_path(&self) -> Option<&Rc<str>> {
        self.script.as_ref()?.source_path.as_ref()
    }

    /// The stand-in package file its source comes from.
    #[inline]
    pub fn stand_in(&self) -> Option<&Rc<str>> {
        self.script.as_ref()?.stand_in.as_ref()
    }

    /// Its [`ScriptSource`], made when it has none, to change.
    pub fn script_mut(&mut self) -> &mut ScriptSource {
        self.script.get_or_insert_with(Default::default)
    }

    pub fn set_source(&mut self, source: Option<Rc<[u8]>>) {
        if source.is_some() || self.script.is_some() {
            self.script_mut().source = source;
        }
    }

    pub fn set_source_path(&mut self, path: Option<Rc<str>>) {
        if path.is_some() || self.script.is_some() {
            self.script_mut().source_path = path;
        }
    }

    pub fn set_stand_in(&mut self, stand_in: Option<Rc<str>>) {
        if stand_in.is_some() || self.script.is_some() {
            self.script_mut().stand_in = stand_in;
        }
    }

    #[inline]
    pub fn has(&self, flag: u16) -> bool {
        self.flags & flag != 0
    }

    #[inline]
    pub fn set_flag(&mut self, flag: u16, on: bool) {
        if on {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }

    /// The explicitly stored value of a property (`Some(Value::Nil)` for a stored nil).
    #[inline]
    pub fn stored(&self, name: NameId) -> Option<&Value> {
        self.props.get(name)
    }

    /// Stores a property value; returns the previously stored one.
    pub fn store(&mut self, name: NameId, value: Value) -> Option<Value> {
        if value.references_instance() {
            self.flags |= flags::HOLDS_REFS;
        }
        let props = self.props.own_mut();
        match props.binary_search_by_key(&name, |(n, _)| *n) {
            Ok(index) => Some(std::mem::replace(&mut props[index].1, value)),
            Err(index) => {
                props.insert(index, (name, value));
                None
            }
        }
    }

    pub fn unstore(&mut self, name: NameId) -> Option<Value> {
        self.props.get(name)?;
        let props = self.props.own_mut();
        match props.binary_search_by_key(&name, |(n, _)| *n) {
            Ok(index) => Some(props.remove(index).1),
            Err(_) => None,
        }
    }

    pub fn attribute(&self, name: &[u8]) -> Option<&Value> {
        self.attributes.get(name)
    }

    pub fn callback(&self, name: NameId) -> Option<&mlua::Value> {
        self.callbacks.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }
}

/// An instance's explicitly stored properties, sorted by name: its own, or, for an instance made of a
/// lazily imported model plan's node and its unchanged copies, the node's, read through the import
/// ([`crate::project::PlanProps`]) until the first write copies them (a bundle's made instances held a
/// vector of ~15-25 values each: ~0.44 GB for the 581k instances the mesh effects scene walks).
pub enum Props {
    Own(Vec<(NameId, Value)>),
    Plan(Rc<crate::project::PlanProps>, u32),
}

impl Default for Props {
    fn default() -> Self {
        Props::Own(Vec::new())
    }
}

impl From<Vec<(NameId, Value)>> for Props {
    fn from(props: Vec<(NameId, Value)>) -> Self {
        Props::Own(props)
    }
}

impl std::fmt::Debug for Props {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Props {
    /// The stored value of property `name`.
    #[inline]
    pub fn get(&self, name: NameId) -> Option<&Value> {
        match self {
            Props::Own(props) => props.binary_search_by_key(&name, |(n, _)| *n).ok().map(|index| &props[index].1),
            Props::Plan(table, node) => table.get(*node, name),
        }
    }

    /// (name, value) of each, sorted by name.
    pub fn iter(&self) -> PropsIter<'_> {
        match self {
            Props::Own(props) => PropsIter::Own(props.iter()),
            Props::Plan(table, node) => PropsIter::Plan(table.iter(*node)),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Props::Own(props) => props.len(),
            Props::Plan(..) => self.iter().count(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The values this instance holds a vector of (none while they are read through a plan).
    pub fn capacity(&self) -> usize {
        match self {
            Props::Own(props) => props.capacity(),
            Props::Plan(..) => 0,
        }
    }

    /// The properties as a vector of its own, to change (a plan node's are copied first).
    pub fn own_mut(&mut self) -> &mut Vec<(NameId, Value)> {
        if let Props::Plan(..) = self {
            let own: Vec<(NameId, Value)> = self.iter().map(|(name, value)| (name, value.clone())).collect();
            *self = Props::Own(own);
        }
        match self {
            Props::Own(props) => props,
            Props::Plan(..) => unreachable!(),
        }
    }

    /// What a copy of the instance stores: every property but `skip` (a plan node's are shared on).
    pub fn copied_without(&self, skip: NameId) -> Props {
        match self {
            Props::Plan(table, node) if self.get(skip).is_none() => Props::Plan(table.clone(), *node),
            _ => Props::Own(self.iter().filter(|(name, _)| *name != skip).map(|(name, value)| (name, value.clone())).collect()),
        }
    }
}

impl<'a> IntoIterator for &'a Props {
    type Item = (NameId, &'a Value);
    type IntoIter = PropsIter<'a>;

    fn into_iter(self) -> PropsIter<'a> {
        self.iter()
    }
}

/// [`Props::iter`]
pub enum PropsIter<'a> {
    Own(std::slice::Iter<'a, (NameId, Value)>),
    Plan(crate::project::PlanPropsIter<'a>),
}

impl<'a> Iterator for PropsIter<'a> {
    type Item = (NameId, &'a Value);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PropsIter::Own(iter) => iter.next().map(|(name, value)| (*name, value)),
            PropsIter::Plan(iter) => iter.next(),
        }
    }
}

/// A vector that is one pointer while empty (most instances have no tags or callbacks): 8 bytes in
/// the instance instead of 24, and a block of its own only once it holds something.
pub struct ThinVec<T>(Option<Box<Vec<T>>>);

impl<T> ThinVec<T> {
    /// The vector, made when there is none, to change.
    pub fn vec_mut(&mut self) -> &mut Vec<T> {
        self.0.get_or_insert_with(Default::default)
    }

    pub fn push(&mut self, item: T) {
        self.vec_mut().push(item);
    }

    pub fn remove(&mut self, index: usize) -> T {
        let item = self.vec_mut().remove(index);
        if self.0.as_ref().is_some_and(|items| items.is_empty()) {
            self.0 = None;
        }
        item
    }

    pub fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        if let Some(items) = &mut self.0 {
            items.retain(keep);
            if items.is_empty() {
                self.0 = None;
            }
        }
    }
}

impl<T> Default for ThinVec<T> {
    fn default() -> Self {
        ThinVec(None)
    }
}

impl<T> std::ops::Deref for ThinVec<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }
}

impl<T> std::ops::DerefMut for ThinVec<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        self.0.as_deref_mut().map_or(&mut [], Vec::as_mut_slice)
    }
}

impl<T> From<Vec<T>> for ThinVec<T> {
    fn from(items: Vec<T>) -> Self {
        ThinVec((!items.is_empty()).then(|| Box::new(items)))
    }
}

impl<T: Clone> Clone for ThinVec<T> {
    fn clone(&self) -> Self {
        ThinVec(self.0.clone())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for ThinVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T> IntoIterator for ThinVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.map(|items| *items).unwrap_or_default().into_iter()
    }
}

impl<'a, T> IntoIterator for &'a ThinVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An instance's attributes in insertion order: its own, or, for an instance made of a lazily imported
/// plan node without instance attributes and its unchanged copies, the node's, made from the plan
/// when first read (`project::PlanProps::attributes`) and copied by the first change (a bundle's ~1.1M
/// attributes in the mesh effects scene were converted for every made instance, read or not).
pub enum Attributes {
    Own(ThinVec<(Str, Value)>),
    Plan(Rc<crate::project::PlanProps>, u32),
}

impl Default for Attributes {
    fn default() -> Self {
        Attributes::Own(ThinVec::default())
    }
}

impl From<Vec<(Str, Value)>> for Attributes {
    fn from(items: Vec<(Str, Value)>) -> Self {
        Attributes::Own(items.into())
    }
}

impl std::fmt::Debug for Attributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Attributes {
    /// The (name, value) pairs in insertion order.
    #[inline]
    pub fn as_slice(&self) -> &[(Str, Value)] {
        match self {
            Attributes::Own(items) => items,
            Attributes::Plan(table, node) => table.attributes(*node),
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, (Str, Value)> {
        self.as_slice().iter()
    }

    pub fn get(&self, name: &[u8]) -> Option<&Value> {
        self.iter().find(|(n, _)| n.as_bytes() == name).map(|(_, v)| v)
    }

    pub fn len(&self) -> usize {
        match self {
            Attributes::Own(items) => items.len(),
            Attributes::Plan(table, node) => table.attribute_count(*node),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The attributes as a vector of its own, to change (a plan node's are copied first).
    pub fn own_mut(&mut self) -> &mut ThinVec<(Str, Value)> {
        if let Attributes::Plan(..) = self {
            *self = Attributes::Own(self.as_slice().to_vec().into());
        }
        match self {
            Attributes::Own(items) => items,
            Attributes::Plan(..) => unreachable!(),
        }
    }

    /// What a copy of the instance holds (a plan node's are shared on).
    pub fn copied(&self) -> Attributes {
        match self {
            Attributes::Own(items) => Attributes::Own(items.clone()),
            Attributes::Plan(table, node) => Attributes::Plan(table.clone(), *node),
        }
    }
}

impl<'a> IntoIterator for &'a Attributes {
    type Item = &'a (Str, Value);
    type IntoIter = std::slice::Iter<'a, (Str, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

struct Slot {
    generation: u32,
    instance: Option<Instance>,
}

/// Per-class default values (Reflection.Default), computed when a class is first instantiated.
#[derive(Default)]
pub struct Defaults {
    per_class: Vec<Option<Box<FxHashMap<NameId, Value>>>>,
}

impl Defaults {
    /// Reflection.Default(Class, Name) for every name the class records or declares.
    pub fn ensure(&mut self, reflection: &Reflection, class: ClassId) {
        if self.per_class.len() < reflection.class_count() {
            self.per_class.resize_with(reflection.class_count(), || None);
        }
        if self.per_class[class as usize].is_some() {
            return;
        }
        let mut names: Vec<NameId> = Vec::new();
        let mut current = Some(class);
        while let Some(id) = current {
            let info = reflection.class(id);
            if let Some(raw) = &info.raw_defaults {
                names.extend(raw.keys().copied());
            }
            current = info.superclass;
        }
        names.extend(reflection.class(class).properties.keys().copied());
        crate::sort::ord(&mut names);
        names.dedup();
        let mut table = FxHashMap::default();
        for name in names {
            if let Some(value) = compute_default(reflection, class, name) {
                table.insert(name, value);
            }
        }
        self.per_class[class as usize] = Some(Box::new(table));
    }

    #[inline]
    pub fn get(&self, class: ClassId, name: NameId) -> Option<&Value> {
        self.per_class.get(class as usize)?.as_ref()?.get(&name)
    }
}

/// Reflection.Default(Class, Name): what a new instance of `class` reads for `name` (`Reflection::default_value`).
pub(crate) fn compute_default(reflection: &Reflection, class: ClassId, name: NameId) -> Option<Value> {
    let info = reflection.class(class);
    let mut value = info
        .raw_defaults
        .as_ref()
        .and_then(|raw| raw.get(&name))
        .and_then(|raw| super::value::decode_default(reflection, raw));
    if value.is_none() {
        let mut ancestor = info.superclass;
        while let (Some(id), None) = (ancestor, &value) {
            let ancestor_info = reflection.class(id);
            if ancestor_info.raw_defaults.as_ref().is_some_and(|raw| raw.contains_key(&name)) {
                value = compute_default(reflection, id, name);
            }
            ancestor = ancestor_info.superclass;
        }
    }
    // (a class whose defaults record no Name is named after itself, not the empty string a string property's zero is:
    // Player, the services and the abstract classes)
    if value.is_none() && name == K::Name as NameId {
        return Some(Value::String(Str::from(&*info.name)));
    }
    if value.is_none() {
        if let Some(property) = reflection.class_property(class, name) {
            value = super::value::zero_value(
                reflection,
                property.script_type,
                &property.script_type_name,
                property.enum_type,
            );
        }
    }
    value
}

/// Unpinned instances remembered between collections (an engine without the Instance API's collector never drains
/// them)
const UNPINNED_CAP: usize = 1 << 16;

#[derive(Default)]
pub struct Dom {
    slots: Vec<Slot>,
    /// Freed slots, reused by `create` (their generation was bumped)
    free_slots: Vec<u32>,
    /// Engine-held references that keep an instance (and its subtree root) from being collected
    pins: FxHashMap<InstanceId, u32>,
    /// Instances whose last pin was released since `take_unpinned` (lua::raw::collect looks at their trees again:
    /// nothing else signals that a tween ended on an instance already outside the DataModel)
    unpinned: Vec<InstanceId>,
    next_serial: u32,
    pub defaults: Defaults,
    /// Bumped on every parent change anywhere (structure version)
    pub structure_version: u64,
    /// Bumped on every property, attribute or tag change anywhere (what the world's query cache
    /// checks, with the structure version)
    pub write_version: u64,
    /// Bumped on every name change anywhere (a child looked up by name reads its parent's child list and its children's names:
    /// the boundary profiler stamps such a read with both)
    pub name_version: u32,
    /// Models whose rig animation caches (animation::pose), with a version a change inside one bumps: a child
    /// attached or detached below it, or a Name / Part0 / Part1 / Scale written on it or below it
    rig_watch: FxHashMap<InstanceId, u32>,
    rig_props: [NameId; 4],
    /// Live slots of the classes someone asked to enumerate cheaply (`instances_of_class`), in
    /// slot order
    tracked: FxHashMap<ClassId, std::collections::BTreeSet<u32>>,
    /// What changed under the subtree the spatial index keeps, while it keeps one (world/broadphase.rs)
    pub watch: Option<Watch>,
    /// Each instance's home, by slot, once [`Dom::keep_homes`] named the home classes: its nearest ancestor of one of them (the
    /// WorldRoots and JointsService, where world/joints.rs and world/gravity.rs ask which world a part or a joint is in), kept
    /// as the tree changes so that question costs one lookup however deep the instance is
    homes: Vec<Option<InstanceId>>,
    /// What is above each home instance's home, by slot, kept with `homes` ([`Dom::homes_above`]): the outermost home it leads to
    /// and how many homes are above it, so whether any instance is in the Workspace, and how many WorldModels are above it, cost
    /// one lookup via its home however many WorldModels are nested above it
    above: Vec<Above>,
    /// Direct child home instances under each home instance (`homes[child] == Some(parent)`), so updating `above` below a moved
    /// home walks only nested homes rather than non-home descendants
    home_children: FxHashMap<InstanceId, Vec<InstanceId>>,
    /// Whether each class (by id) is a home class; empty while homes are not kept
    home_classes: Vec<bool>,
    /// Whether each class (by id) is one a [`WorldLog`] logs below an instance leaving its world, and whose writes it logs (the
    /// BaseParts but Terrain: [`Dom::keep_logged_classes`]); empty while none is named
    logged_classes: Vec<bool>,
    /// The properties no spatial index reads, the Workspace's or a WorldModel's ([`Dom::keep_unread`]: a part's collision group):
    /// a write of one leaves the instance out of the [`Watch`] and of its world's [`WorldLog`] alike ([`Dom::bump_version_of`])
    unread: Vec<NameId>,
    /// A stack reused to walk a subtree whose home changed ([`Dom::spread_home`])
    home_walk: Vec<InstanceId>,
    /// A stack reused to walk nested homes whose outermost home or depth changed ([`Dom::spread_above`])
    above_walk: Vec<InstanceId>,
    /// A stack reused to walk a subtree that leaves a followed world ([`Dom::log_world_leaving`])
    world_walk: Vec<InstanceId>,
    /// What changed in each world a spatial index of its own follows, by its home ([`Dom::follow_world`]: a WorldModel's,
    /// world/broadphase.rs `WorldIndex`)
    world_logs: FxHashMap<InstanceId, WorldLog>,
    /// The instances of the classes [`Dom::log_homes`] names (by id; empty while none is) whose home changed since
    /// [`Dom::take_home_log`] last took them, logged by whatever changed it (world/joints.rs: which rigid joints are in a world),
    /// and whether more changed than the log keeps (twice the slots and 4,096 more), which it then stops logging
    home_logged: Vec<bool>,
    home_log: Vec<InstanceId>,
    home_log_full: bool,
}

/// What is above an instance's home ([`Dom::homes_above`]): the outermost home its homes lead to (the last of them, which is in
/// no home: the Workspace for an instance in it, a WorldModel out of every world, JointsService) and how many homes are above it,
/// that one included. None and 0 for an instance in no home.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Above {
    pub top: Option<InstanceId>,
    pub homes: u32,
}

/// What changed in a world a spatial index of its own follows since it last took the log (world/broadphase.rs `WorldIndex`: a
/// WorldModel's), read off the homes ([`Dom::home`], one lookup a change however deep the instance is), as [`Watch`] logs the
/// Workspace's: the instances attached in it, detached from it (with the parts below them, down to the homes in them, whose
/// worlds are their own: [`Dom::keep_logged_classes`]), put in place of another there or given another class there (`moved`),
/// and the parts written there (`written`, the same classes), each instance once in each until the index takes the log, however
/// often it changed. Past `limit` entries the log stops and says so (`overflowed`), and the index starts over from a walk of
/// its world, so a run that stops querying it keeps no more than that.
#[derive(Default)]
pub struct WorldLog {
    pub moved: Vec<InstanceId>,
    pub written: Vec<InstanceId>,
    pub limit: usize,
    pub overflowed: bool,
    /// The instances in `moved`, and those in `written`
    logged: FxHashSet<InstanceId>,
    written_logged: FxHashSet<InstanceId>,
}

impl WorldLog {
    /// What the log holds, `moved` and `written`, taken by the index: the log starts again with nothing in it.
    pub fn take(&mut self) -> (Vec<InstanceId>, Vec<InstanceId>) {
        clear_set(&mut self.logged);
        clear_set(&mut self.written_logged);
        (std::mem::take(&mut self.moved), std::mem::take(&mut self.written))
    }
}

/// What changed under `root` (the Workspace) since the spatial index last took the log (world/broadphase.rs): the instances
/// attached below it, detached from it (with the indexed instances below one, which may leave it later without passing through
/// the root: Destroy takes a model out first), put in place of another there or given another class there, and the indexed
/// instances freed (`moved`), and the indexed instances written (`written`), each instance once until the index takes the log
/// (`moved`: [`Watch::logged`]; `written`: [`flags::LOGGED`]), however often it moves or is written. Each entry is a few ids of
/// work for the index, so a change costs it what the change touched, never a walk of the root. Past `limit` entries (distinct
/// instances) the log stops and says so (`overflowed`), and the index starts over from the root, so a run that stops querying
/// keeps no more than that. A write of a property the index never reads ([`Dom::keep_unread`]) is not logged.
pub struct Watch {
    pub root: InstanceId,
    pub moved: Vec<InstanceId>,
    pub written: Vec<InstanceId>,
    pub limit: usize,
    pub overflowed: bool,
    /// The instances in `moved` (a stale id stays apart from its slot's next instance)
    logged: FxHashSet<InstanceId>,
    /// A leaving subtree's walk (Dom::log_leaving)
    walk: Vec<InstanceId>,
}

impl Watch {
    pub fn new(root: InstanceId, limit: usize) -> Self {
        Watch { root, moved: Vec::new(), written: Vec::new(), limit, overflowed: false, logged: FxHashSet::default(), walk: Vec::new() }
    }

    fn room(&mut self) -> bool {
        if !self.overflowed && self.moved.len() + self.written.len() >= self.limit {
            self.overflowed = true;
            self.moved = Vec::new();
            self.written = Vec::new();
            self.logged = FxHashSet::default();
        }
        !self.overflowed
    }

    /// What the log holds, `moved` and `written`, taken by the index: the log starts again with nothing in it (the caller clears
    /// the written instances' [`flags::LOGGED`]).
    pub fn take(&mut self) -> (Vec<InstanceId>, Vec<InstanceId>) {
        clear_set(&mut self.logged);
        (std::mem::take(&mut self.moved), std::mem::take(&mut self.written))
    }
}

/// Empties a set that is filled again and emptied once a round (Watch::logged at each take of the log, the broadphase's walk
/// marks at each sync, the parts waiting for the spatial index each time it takes them in): in place while its table is near
/// what it held, else for a table the size of what it held. A clear writes every slot of the table, and a table keeps what a
/// burst grew it to, so after a million instances moved before one query each later round of one move and one query cleared
/// tables of two million slots (30 us of host time a round, 1.9 us before the burst); a round's clear now costs what it held.
/// Only a set read for membership is emptied so: a new table lists what it holds in another order.
pub(crate) fn clear_set<T>(set: &mut FxHashSet<T>) {
    let held = set.len();
    if outgrown(set.capacity(), held) {
        *set = FxHashSet::with_capacity_and_hasher(held, Default::default());
    } else {
        set.clear();
    }
}

/// Empties a map read by key alone that is filled again and emptied once a round, as [`clear_set`] empties a set, where the
/// round may take entries out before it ends (services/physics.rs Groups::unsent, at each loopback flush): `held` is what the
/// round put in it, the size its table is kept at.
pub(crate) fn clear_map<K, V>(map: &mut FxHashMap<K, V>, held: usize) {
    if outgrown(map.capacity(), held) {
        *map = FxHashMap::with_capacity_and_hasher(held, Default::default());
    } else {
        map.clear();
    }
}

/// Whether a table with room for `capacity` entries that held `held` in its round is replaced rather than cleared ([`clear_set`])
fn outgrown(capacity: usize, held: usize) -> bool {
    capacity > 4 * held.max(1024)
}

impl Dom {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an unparented instance of `class` with the next serial (InstanceModule.Create).
    pub fn create(&mut self, reflection: &Reflection, class: ClassId) -> InstanceId {
        self.next_serial += 1;
        self.create_with_serial(reflection, class, self.next_serial)
    }

    /// Reserves `count` serials for instances made later ([`Dom::create_with_serial`]), as if they
    /// were made now; returns the first.
    pub fn reserve_serials(&mut self, count: u32) -> u32 {
        let first = self.next_serial + 1;
        self.next_serial += count;
        first
    }

    /// Creates an unparented instance of `class` with a serial [`Dom::reserve_serials`] reserved.
    pub fn create_with_serial(&mut self, reflection: &Reflection, class: ClassId, serial: u32) -> InstanceId {
        self.defaults.ensure(reflection, class);
        let name = match self.defaults.get(class, K::Name as NameId) {
            Some(Value::String(name)) => name.clone(),
            _ => Str::from(&*reflection.class(class).name),
        };
        let instance = Instance {
            class,
            serial,
            name,
            parent: None,
            children: Children::default(),
            place: 0,
            props: Props::default(),
            attributes: Attributes::default(),
            tags: ThinVec::default(),
            flags: 0,
            script: None,
            studio_node: StudioNode::Unknown,
            signals: None,
            callbacks: ThinVec::default(),
            unmodeled: None,
            version: 0,
            child_version: 0,
        };
        if let Some(index) = self.free_slots.pop() {
            let slot = &mut self.slots[index as usize];
            slot.instance = Some(instance);
            if let Some(set) = self.tracked.get_mut(&class) {
                set.insert(index);
            }
            if let Some(home) = self.homes.get_mut(index as usize) {
                *home = None;
                self.above[index as usize] = Above::default();
            }
            return InstanceId { index, generation: slot.generation };
        }
        let index = self.slots.len() as u32;
        self.slots.push(Slot { generation: 0, instance: Some(instance) });
        if !self.home_classes.is_empty() {
            self.homes.push(None);
            self.above.push(Above::default());
        }
        if let Some(set) = self.tracked.get_mut(&class) {
            set.insert(index);
        }
        InstanceId { index, generation: 0 }
    }

    /// The live instances of exactly `class`, in slot order (the order of a walk over every
    /// slot). The first call for a class scans the arena; later ones read the index `create` /
    /// `free` keep for it.
    pub fn instances_of_class(&mut self, class: ClassId) -> Vec<InstanceId> {
        self.track_class(class);
        self.tracked[&class].iter().filter_map(|&index| self.id_at_slot(index)).collect()
    }

    /// How many live instances of exactly `class` there are (the index [`Dom::instances_of_class`] keeps).
    pub fn count_of_class(&mut self, class: ClassId) -> usize {
        self.track_class(class);
        self.tracked[&class].len()
    }

    /// Gives a live instance another class in place (a bundle's placeholder becoming the bundle's root:
    /// project::plan), keeping its id, serial, stored values and children: the class index and the
    /// class's defaults follow, and the change counts as one to its properties and the tree.
    pub fn set_class(&mut self, reflection: &Reflection, id: InstanceId, class: ClassId) {
        let old = self.at(id).class;
        if old == class {
            return;
        }
        self.defaults.ensure(reflection, class);
        let was_home = self.is_home(id);
        if let Some(set) = self.tracked.get_mut(&old) {
            set.remove(&id.index);
        }
        if let Some(set) = self.tracked.get_mut(&class) {
            set.insert(id.index);
        }
        let default_name = match self.defaults.get(class, K::Name as NameId) {
            Some(Value::String(name)) => name.clone(),
            _ => Str::from(&*reflection.class(class).name),
        };
        let instance = self.at_mut(id);
        instance.class = class;
        let renamed = instance.stored(K::Name as NameId).is_none() && instance.name != default_name;
        if instance.stored(K::Name as NameId).is_none() {
            instance.name = default_name;
        }
        instance.version = instance.version.wrapping_add(1);
        if renamed {
            self.name_version = self.name_version.wrapping_add(1);
        }
        self.structure_version += 1;
        self.write_version += 1;
        if self.watched(id) {
            self.log_moved(id);
        }
        // (an instance that takes or leaves a logged class is logged as if its home changed: world/joints.rs works its joints out
        // again)
        if !self.home_logged.is_empty() {
            let logged = |class: ClassId| self.home_logged.get(class as usize).copied().unwrap_or(false);
            let class = if logged(class) { class } else { old };
            self.log_home(id, class);
        }
        if !self.world_logs.is_empty() {
            // (and what is below it, whose world it takes or gives back when it becomes or stops being a home; one that stops
            // being one drops its own log, so its index, if it becomes one again, starts over from a walk)
            let home = self.home(id);
            self.log_world_leaving(home, id, self.is_home(id) != was_home);
            if was_home && !self.is_home(id) {
                self.world_logs.remove(&id);
            }
        }
        // (one that becomes or stops being a WorldRoot moves its descendants' world)
        if self.is_home(id) != was_home {
            let old_home = self.home(id);
            if was_home {
                if let Some(parent_home) = old_home {
                    self.remove_home_child(parent_home, id);
                }
                self.above[id.index as usize] = Above::default();
                self.spread_home(id, old_home);
                self.home_children.remove(&id);
            } else {
                self.above[id.index as usize] = self.above_below(old_home);
                if let Some(parent_home) = old_home {
                    self.home_children.entry(parent_home).or_default().push(id);
                }
                self.spread_home(id, Some(id));
            }
        }
    }

    /// Keeps each instance's home from now on ([`Dom::home`]): its nearest ancestor of a class `is_home` (by class id) marks.
    /// Those of the instances that exist are worked out now, once.
    pub fn keep_homes(&mut self, is_home: Vec<bool>) {
        self.home_classes = is_home;
        self.homes.clear();
        self.homes.resize(self.slots.len(), None);
        self.above.clear();
        self.above.resize(self.slots.len(), Above::default());
        self.home_children.clear();
        let mut walk: Vec<(InstanceId, Option<InstanceId>)> =
            (0..self.slots.len() as u32).filter_map(|slot| self.id_at_slot(slot)).filter(|&id| self.at(id).parent.is_none()).map(|root| (root, None)).collect();
        while let Some((node, home)) = walk.pop() {
            // (a home is above each instance below it, so it was worked out before them)
            self.homes[node.index as usize] = home;
            let is_home = self.is_home(node);
            if is_home {
                self.above[node.index as usize] = self.above_below(home);
                if let Some(parent_home) = home {
                    self.home_children.entry(parent_home).or_default().push(node);
                }
            }
            let below = if is_home { Some(node) } else { home };
            walk.extend(self.at(node).children.iter().map(|&child| (child, below)));
        }
    }

    /// What is above an instance whose home is `home`: the outermost home `home` leads to (itself when it is in no home), and one
    /// home more than is above `home`.
    #[inline]
    fn above_below(&self, home: Option<InstanceId>) -> Above {
        match home {
            None => Above::default(),
            Some(home) => {
                let up = self.above[home.index as usize];
                Above { top: Some(up.top.unwrap_or(home)), homes: up.homes + 1 }
            }
        }
    }

    /// The outermost home above the live instance `id` and how many homes are above it ([`Above`]), found without walking its
    /// ancestors or its homes: none and 0 while homes are not kept.
    #[inline]
    pub fn homes_above(&self, id: InstanceId) -> Above {
        self.above_below(self.home(id))
    }

    /// The nearest ancestor of the live instance `id` of a home class ([`Dom::keep_homes`]: the WorldRoots and JointsService),
    /// found without walking its ancestors; None when it has none, and for every instance while homes are not kept.
    #[inline]
    pub fn home(&self, id: InstanceId) -> Option<InstanceId> {
        self.homes.get(id.index as usize).copied().flatten()
    }

    /// Whether the dom keeps homes ([`Dom::keep_homes`]): [`Dom::home`] and [`Dom::homes_above`] answer none until it does.
    pub fn keeps_homes(&self) -> bool {
        !self.home_classes.is_empty()
    }

    /// The next ancestor of `id` to look at for one of a home class: its home while the dom keeps homes, else its parent.
    /// Followed from an instance, it meets every ancestor of a home class, nearest first: one step per home above a kept
    /// instance, however deep it is (instance.rs's walk to a part's nearest WorldRoot takes one or two).
    #[inline]
    pub fn home_step(&self, id: InstanceId) -> Option<InstanceId> {
        if self.home_classes.is_empty() { self.at(id).parent } else { self.home(id) }
    }

    /// Whether the live instance `id` is a descendant of `ancestor` ([`Dom::is_descendant_of`]'s answer), read from the homes
    /// when `ancestor` is of a home class (the Workspace, a WorldModel) and the dom keeps them: one lookup when `ancestor` is in
    /// no home itself (the Workspace, JointsService, a WorldModel out of every world: whether it is the outermost home above
    /// `id`), however deep `id` is and however many WorldModels are nested above it; one lookup per home between them for a home
    /// inside another (no engine check asks one). Any other ancestor, a stale one included, is looked for up the tree (as is a
    /// stale `id`, which panics there).
    pub fn is_descendant_of_home(&self, id: InstanceId, ancestor: InstanceId) -> bool {
        let home_class = |node: &Instance| self.home_classes.get(node.class as usize).copied().unwrap_or(false);
        if !self.get(ancestor).is_some_and(home_class) || self.get(id).is_none() {
            return self.is_descendant_of(id, ancestor);
        }
        let top = self.homes_above(id).top;
        if self.home(ancestor).is_none() || top != self.homes_above(ancestor).top {
            return top == Some(ancestor);
        }
        let mut home = self.home(id);
        while let Some(node) = home {
            if node == ancestor {
                return true;
            }
            home = self.home(node);
        }
        false
    }

    /// Whether `id` is of a home class ([`Dom::keep_homes`]: a WorldRoot or JointsService), so its descendants' homes stop at it.
    pub fn is_home(&self, id: InstanceId) -> bool {
        !self.home_classes.is_empty() && self.home_classes.get(self.at(id).class as usize).copied().unwrap_or(false)
    }

    /// The home of a child of `parent`: `parent` itself when it is of a home class, else `parent`'s home.
    fn home_below(&self, parent: InstanceId) -> Option<InstanceId> {
        if self.is_home(parent) { Some(parent) } else { self.home(parent) }
    }

    /// Logs each instance of a class `logged` marks (by class id) whose home changes from now on, whatever changes it
    /// ([`Dom::take_home_log`]).
    pub fn log_homes(&mut self, logged: Vec<bool>) {
        self.home_logged = logged;
    }

    /// The logged instances whose home changed since the last take, in the order they changed ([`Dom::log_homes`]): one can be
    /// there more than once, or freed since. None when more changed than the log keeps: then any of them may have.
    pub fn take_home_log(&mut self) -> Option<Vec<InstanceId>> {
        if std::mem::take(&mut self.home_log_full) {
            return None;
        }
        Some(std::mem::take(&mut self.home_log))
    }

    fn remove_home_child(&mut self, parent: InstanceId, child: InstanceId) {
        if let Some(children) = self.home_children.get_mut(&parent) {
            if let Some(pos) = children.iter().position(|&id| id == child) {
                children.swap_remove(pos);
            }
            if children.is_empty() {
                self.home_children.remove(&parent);
            }
        }
    }

    /// Gives the live instance `id` the home `home`, logged when it is another and its class is ([`Dom::log_homes`]), and what is
    /// above it with it when `id` is a home; whether what is above it changed.
    #[inline]
    fn put_home(&mut self, id: InstanceId, home: Option<InstanceId>) -> bool {
        let slot = id.index as usize;
        let old = self.homes[slot];
        let is_home = self.is_home(id);
        if old != home {
            self.homes[slot] = home;
            self.log_home(id, self.at(id).class);
            if is_home {
                if let Some(old_home) = old {
                    self.remove_home_child(old_home, id);
                }
                if let Some(new_home) = home {
                    self.home_children.entry(new_home).or_default().push(id);
                }
            }
        }
        if is_home { self.put_above(id) } else { false }
    }

    /// Logs `id` when `class` is logged ([`Dom::log_homes`]): its home changed, or it took or left that class.
    #[inline]
    fn log_home(&mut self, id: InstanceId, class: ClassId) {
        if self.home_logged.get(class as usize).copied().unwrap_or(false) && !self.home_log_full {
            if self.home_log.len() < 2 * self.slots.len() + 4096 {
                self.home_log.push(id);
            } else {
                self.home_log_full = true;
                self.home_log = Vec::new();
            }
        }
    }

    /// Works out again what is above the live instance `id` from its home's; whether it changed.
    #[inline]
    fn put_above(&mut self, id: InstanceId) -> bool {
        let above = self.above_below(self.homes[id.index as usize]);
        std::mem::replace(&mut self.above[id.index as usize], above) != above
    }

    /// `child`'s home now that its parent changed: when it is another than it was, `child` and the descendants that take theirs
    /// from it get the new one (a walk of the subtree down to the homes in it), and when that moves `child` or a home below it
    /// to another outermost home, or under more or fewer homes, everything below that home is told so too (the rest of the
    /// subtree). A subtree moved within its world walks nothing.
    fn rehome(&mut self, child: InstanceId) {
        if self.home_classes.is_empty() {
            return;
        }
        let home = self.at(child).parent.and_then(|parent| self.home_below(parent));
        if self.homes[child.index as usize] == home {
            return;
        }
        let moved = self.put_home(child, home);
        if !self.is_home(child) {
            self.spread_home(child, home);
        } else if moved {
            self.spread_above(child);
        }
    }

    /// Gives `home` to the descendants of `id` down to and including the instances of a home class (below which the homes stay),
    /// and updates `above` on every nested home below one whose own changed.
    fn spread_home(&mut self, id: InstanceId, home: Option<InstanceId>) {
        let mut walk = std::mem::take(&mut self.home_walk);
        walk.clear();
        walk.extend(self.at(id).children.iter().copied());
        while let Some(node) = walk.pop() {
            let moved = self.put_home(node, home);
            if !self.is_home(node) {
                walk.extend(self.at(node).children.iter().copied());
            } else if moved {
                self.spread_above(node);
            }
        }
        self.home_walk = walk;
    }

    /// Works out again what is above each descendant home of the home `id` (whose own changed), their homes staying.
    fn spread_above(&mut self, id: InstanceId) {
        let mut walk = std::mem::take(&mut self.above_walk);
        walk.clear();
        if let Some(children) = self.home_children.get(&id) {
            walk.extend_from_slice(children);
        }
        while let Some(node) = walk.pop() {
            if self.put_above(node)
                && let Some(children) = self.home_children.get(&node)
            {
                walk.extend_from_slice(children);
            }
        }
        self.above_walk = walk;
    }

    fn track_class(&mut self, class: ClassId) {
        if !self.tracked.contains_key(&class) {
            let set = (0..self.slots.len() as u32)
                .filter(|&index| self.slots[index as usize].instance.as_ref().is_some_and(|i| i.class == class))
                .collect();
            self.tracked.insert(class, set);
        }
    }

    /// Frees an instance's slot (garbage collection: the caller made sure nothing references
    /// it). The slot's generation is bumped, so the id and every copy of it become stale.
    pub fn free(&mut self, id: InstanceId) -> Option<Instance> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let instance = slot.instance.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        if let Some(set) = self.tracked.get_mut(&instance.class) {
            set.remove(&id.index);
        }
        // (the index drops what it held of it: the stale id says so)
        if instance.flags & flags::INDEXED != 0 {
            self.log_moved(id);
        }
        self.free_slots.push(id.index);
        self.pins.remove(&id);
        self.rig_watch.remove(&id);
        self.world_logs.remove(&id);
        if !self.home_classes.is_empty() {
            if self.home_classes.get(instance.class as usize).copied().unwrap_or(false) {
                if let Some(old_home) = self.homes[id.index as usize] {
                    self.remove_home_child(old_home, id);
                }
                self.home_children.remove(&id);
            }
            self.homes[id.index as usize] = None;
            self.above[id.index as usize] = Above::default();
        }
        Some(instance)
    }

    /// Keeps `id` alive while the engine holds it outside the tree (a playing tween, a tween's
    /// target, ...). Pins count: each `pin` needs one `unpin`.
    pub fn pin(&mut self, id: InstanceId) {
        if self.get(id).is_some() {
            *self.pins.entry(id).or_default() += 1;
        }
    }

    pub fn unpin(&mut self, id: InstanceId) {
        if let Some(count) = self.pins.get_mut(&id) {
            *count -= 1;
            if *count == 0 {
                self.pins.remove(&id);
                if self.unpinned.len() < UNPINNED_CAP {
                    self.unpinned.push(id);
                }
            }
        }
    }

    /// The instances unpinned since the last call.
    pub fn take_unpinned(&mut self) -> Vec<InstanceId> {
        std::mem::take(&mut self.unpinned)
    }

    pub fn unpinned_len(&self) -> usize {
        self.unpinned.len()
    }

    #[inline]
    pub fn is_pinned(&self, id: InstanceId) -> bool {
        self.pins.contains_key(&id)
    }

    /// (property values stored, their vectors' capacity, child ids' capacity, attributes, instances
    /// with signals) over every live instance (diagnostics)
    pub fn storage(&self) -> (usize, usize, usize, usize, usize) {
        let mut total = (0, 0, 0, 0, 0);
        for instance in self.slots.iter().filter_map(|slot| slot.instance.as_ref()) {
            total.0 += instance.props.len();
            total.1 += instance.props.capacity();
            total.2 += instance.children.capacity();
            total.3 += instance.attributes.len();
            total.4 += instance.signals.is_some() as usize;
        }
        total
    }

    /// Number of live instances.
    pub fn live_count(&self) -> usize {
        self.slots.len() - self.free_slots.len()
    }

    /// The live instances by where they are: (in the tree under `root`, the DataModel, itself included; alive outside it;
    /// destroyed, kept until the VM collects the last userdata that holds them)
    pub fn census(&self, root: Option<InstanceId>) -> (u64, u64, u64) {
        // (per slot: 0 not known yet, 1 under root, 2 not; a walk up stops at the first known ancestor and marks its path)
        let mut under = vec![0u8; self.slots.len()];
        if let Some(slot) = root.filter(|root| self.get(*root).is_some()).and_then(|root| under.get_mut(root.index as usize)) {
            *slot = 1;
        }
        let (mut inside, mut outside, mut destroyed) = (0, 0, 0);
        let mut path: Vec<usize> = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(instance) = slot.instance.as_ref() else { continue };
            if instance.has(flags::DESTROYED) {
                destroyed += 1;
                continue;
            }
            if under[index] == 0 {
                path.clear();
                path.push(index);
                let mut parent = instance.parent;
                let found = loop {
                    let Some(id) = parent else { break 2 };
                    let at = id.index as usize;
                    if under.get(at).copied().unwrap_or(2) != 0 {
                        break under[at];
                    }
                    path.push(at);
                    parent = self.get(id).and_then(|instance| instance.parent);
                };
                for &at in &path {
                    under[at] = found;
                }
            }
            if under[index] == 1 { inside += 1 } else { outside += 1 }
        }
        (inside, outside, destroyed)
    }

    /// Number of instances ever created (the last serial).
    pub fn created(&self) -> u32 {
        self.next_serial
    }

    #[inline]
    pub fn get(&self, id: InstanceId) -> Option<&Instance> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.instance.as_ref()
    }

    #[inline]
    pub fn get_mut(&mut self, id: InstanceId) -> Option<&mut Instance> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.instance.as_mut()
    }

    /// The instance behind a live id (panics on a stale id: an engine bug).
    #[inline]
    pub fn at(&self, id: InstanceId) -> &Instance {
        self.get(id).expect("stale instance id")
    }

    #[inline]
    pub fn at_mut(&mut self, id: InstanceId) -> &mut Instance {
        self.get_mut(id).expect("stale instance id")
    }

    /// The id stored in slot `index` (for walks keyed by slot).
    pub fn id_at_slot(&self, index: u32) -> Option<InstanceId> {
        let slot = self.slots.get(index as usize)?;
        slot.instance.as_ref()?;
        Some(InstanceId { index, generation: slot.generation })
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// RawGet: the stored value, else the class default (`None`: nil).
    #[inline]
    pub fn raw_get(&self, id: InstanceId, name: NameId) -> Option<&Value> {
        let instance = self.at(id);
        match instance.stored(name) {
            Some(Value::Nil) => None,
            Some(value) => Some(value),
            None => self.defaults.get(instance.class, name),
        }
    }

    /// RawGet as a number (0 for nil or other types).
    #[inline]
    pub fn number(&self, id: InstanceId, name: NameId) -> Option<f64> {
        self.raw_get(id, name).and_then(Value::as_number)
    }

    #[inline]
    pub fn boolean(&self, id: InstanceId, name: NameId) -> Option<bool> {
        self.raw_get(id, name).and_then(Value::as_bool)
    }

    #[inline]
    pub fn class_of(&self, id: InstanceId) -> ClassId {
        self.at(id).class
    }

    #[inline]
    pub fn parent_of(&self, id: InstanceId) -> Option<InstanceId> {
        self.at(id).parent
    }

    #[inline]
    pub fn children_of(&self, id: InstanceId) -> &Children {
        &self.at(id).children
    }

    /// Where `id` is among its siblings: its place in its parent's children, so it orders siblings as GetChildren lists them
    /// without looking for it in the list.
    #[inline]
    pub fn sibling_place(&self, id: InstanceId) -> u32 {
        self.at(id).place
    }

    /// Starts watching `model` for the rig cache (the properties that change a rig are `props`); its version now.
    pub fn watch_rig(&mut self, model: InstanceId, props: [NameId; 4]) -> u32 {
        self.rig_props = props;
        *self.rig_watch.entry(model).or_insert(0)
    }

    /// `from` and its ancestors that are watched rigs: something below them changed.
    fn touch_rigs(&mut self, from: InstanceId) {
        if self.rig_watch.is_empty() {
            return;
        }
        let mut current = Some(from);
        while let Some(id) = current {
            if let Some(version) = self.rig_watch.get_mut(&id) {
                *version = version.wrapping_add(1);
            }
            current = self.get(id).and_then(|instance| instance.parent);
        }
    }

    /// A property of `id` was written: a watched rig changes when it is one of those that make a rig.
    pub fn touch_rigs_for_property(&mut self, id: InstanceId, name: NameId) {
        if !self.rig_watch.is_empty() && self.rig_props.contains(&name) {
            self.touch_rigs(id);
        }
    }

    /// Appends `child` to `parent`'s children and sets its parent (no events: RawParent).
    pub fn attach(&mut self, child: InstanceId, parent: InstanceId) {
        self.link(child, parent);
        self.rehome(child);
    }

    /// Removes `child` from its parent's children and clears its parent (no events). How many places a client's search for it
    /// passes ([`Dom::reparent`]).
    pub fn detach(&mut self, child: InstanceId) -> usize {
        let searched = self.unlink(child);
        self.rehome(child);
        searched
    }

    /// [`Dom::detach`], then [`Dom::attach`] to `parent` when there is one, with `child`'s home worked out once: a subtree moved
    /// within its world walks nothing for its homes. Returns how many places a playing client's search for `child` passes to take
    /// it out of its old parent's list ([`Children`]): those before it, none for the last (a client looks there first) or for a
    /// child with no parent. A Parent write prices them (cost.rs CHILD_SEARCHED); the dom itself takes the child out from its
    /// place.
    pub fn reparent(&mut self, child: InstanceId, parent: Option<InstanceId>) -> usize {
        let searched = self.unlink(child);
        if let Some(parent) = parent {
            self.link(child, parent);
        }
        self.rehome(child);
        searched
    }

    fn link(&mut self, child: InstanceId, parent: InstanceId) {
        self.touch_rigs(parent);
        let parent_instance = self.at_mut(parent);
        let place = parent_instance.children.slots.len() as u32;
        parent_instance.children.slots.push(child);
        parent_instance.child_version = parent_instance.child_version.wrapping_add(1);
        let instance = self.at_mut(child);
        instance.parent = Some(parent);
        instance.place = place;
        self.structure_version += 1;
        if self.watched(parent) {
            self.log_moved(child);
        }
        if !self.world_logs.is_empty() {
            let home = self.home_below(parent);
            self.log_world(home, child, false);
        }
    }

    /// Takes `child` out of its parent's children, from its place, as a client does ([`Children::take`]): [`Dom::reparent`]'s
    /// count of what a client's search passes, its place (none for the last).
    fn unlink(&mut self, child: InstanceId) -> usize {
        let Some(parent) = self.at(child).parent else {
            return 0;
        };
        if self.watched(parent) {
            self.log_leaving(child);
        }
        if !self.world_logs.is_empty() {
            let home = self.home_below(parent);
            self.log_world_leaving(home, child, false);
        }
        self.touch_rigs(parent);
        let mut searched = 0;
        if let Some(place) = self.place_in(parent, child) {
            let moved = self.at_mut(parent).children.take(place);
            if !moved.is_empty() {
                searched = place;
            }
            for at in moved {
                let id = self.at(parent).children.slots[at];
                self.at_mut(id).place = at as u32;
            }
        }
        let parent_instance = self.at_mut(parent);
        parent_instance.child_version = parent_instance.child_version.wrapping_add(1);
        self.at_mut(child).parent = None;
        self.structure_version += 1;
        searched
    }

    /// Where `child` is in `parent`'s children: its place (a list someone edited from outside the dom is searched).
    fn place_in(&self, parent: InstanceId, child: InstanceId) -> Option<usize> {
        let place = self.at(child).place as usize;
        let slots = &self.at(parent).children.slots;
        if slots.get(place) == Some(&child) { Some(place) } else { slots.iter().position(|&slot| slot == child) }
    }

    /// Hands `from`'s children to `to`, which has none, in their order and at their places (a project mount putting a script in a
    /// folder's place, before [`Dom::replace_child`]: project.rs ensure_child). Moves nothing's version, as the mount's own edit did.
    pub fn hand_children(&mut self, from: InstanceId, to: InstanceId) {
        let children = std::mem::take(&mut self.at_mut(from).children);
        for &child in &children {
            self.at_mut(child).parent = Some(to);
        }
        self.at_mut(to).children = children;
    }

    /// Puts `parent`'s children in `order`, a permutation of them (rur replays a place's Studio order: its engine/order.rs), each at
    /// its new place. Moves nothing's version: the caller bumps `child_version` and `structure_version` when anything moved.
    pub fn reorder_children(&mut self, parent: InstanceId, order: &[InstanceId]) {
        let mut children = std::mem::take(&mut self.at_mut(parent).children);
        children.slots.clear();
        children.slots.extend_from_slice(order);
        for (place, &child) in children.slots.iter().enumerate() {
            self.at_mut(child).place = place as u32;
        }
        self.at_mut(parent).children = children;
    }

    /// Replaces `old` by `new` in the same child slot of `parent` (project EnsureChild).
    pub fn replace_child(&mut self, parent: InstanceId, old: InstanceId, new: InstanceId) {
        self.touch_rigs(parent);
        let place = self.place_in(parent, old);
        let parent_instance = self.at_mut(parent);
        if let Some(place) = place {
            parent_instance.children.slots[place] = new;
        }
        parent_instance.child_version = parent_instance.child_version.wrapping_add(1);
        self.at_mut(old).parent = None;
        let instance = self.at_mut(new);
        instance.parent = Some(parent);
        if let Some(place) = place {
            instance.place = place as u32;
        }
        self.structure_version += 1;
        if self.watched(parent) {
            self.log_leaving(old);
            self.log_moved(new);
        }
        if !self.world_logs.is_empty() {
            let home = self.home_below(parent);
            // (and the parts below the new child, which leave this world when it is a WorldRoot holding them: handed over)
            self.log_world_leaving(home, old, false);
            self.log_world_leaving(home, new, true);
        }
        self.rehome(old);
        // (the new child holds what was handed to it, the old one's children when a project mount replaces a folder by a script:
        // their homes are worked out again, whatever the new child's own was)
        if !self.home_classes.is_empty() {
            let home = self.home_below(parent);
            self.put_home(new, home);
            let below = if self.is_home(new) { Some(new) } else { home };
            self.spread_home(new, below);
        }
    }

    /// Whether `node` is the watched root or below it.
    fn watched(&self, node: InstanceId) -> bool {
        let Some(watch) = &self.watch else { return false };
        if watch.overflowed {
            return false;
        }
        let root = watch.root;
        let mut current = Some(node);
        while let Some(id) = current {
            if id == root {
                return true;
            }
            current = self.get(id).and_then(|instance| instance.parent);
        }
        false
    }

    /// Logs `id` among the instances that moved, unless it is there already.
    fn log_moved(&mut self, id: InstanceId) {
        if let Some(watch) = &mut self.watch
            && !watch.logged.contains(&id)
            && watch.room()
        {
            watch.logged.insert(id);
            watch.moved.push(id);
        }
    }

    /// Logs `id`, leaving the watched root's subtree, and every indexed instance below it.
    fn log_leaving(&mut self, id: InstanceId) {
        self.log_moved(id);
        let Some(watch) = &mut self.watch else { return };
        let mut walk = std::mem::take(&mut watch.walk);
        walk.clear();
        self.at(id).children.copy_to(&mut walk);
        while let Some(node) = walk.pop() {
            let instance = self.at(node);
            instance.children.copy_to(&mut walk);
            if instance.flags & flags::INDEXED != 0 {
                self.log_moved(node);
            }
        }
        if let Some(watch) = &mut self.watch {
            watch.walk = walk;
        }
    }

    /// Names the classes (by id) a [`WorldLog`] logs below an instance that leaves its world, and whose writes it logs: the
    /// parts a world's index holds, which may leave it later without passing through it (world/mod.rs: the BaseParts but
    /// Terrain).
    pub fn keep_logged_classes(&mut self, classes: Vec<bool>) {
        self.logged_classes = classes;
    }

    /// Names the properties no spatial index reads (world/mod.rs: a part's CollisionGroup and CollisionGroupId), whose writes
    /// [`Dom::bump_version_of`] leaves out of the Workspace's log and every world's, from the run's start: before the Workspace's
    /// index is made too, when a WorldModel's index may already follow its world.
    pub fn keep_unread(&mut self, names: Vec<NameId>) {
        self.unread = names;
    }

    /// Starts (or starts over) the change log of the world whose home is `home` ([`WorldLog`]), up to `limit` entries.
    pub fn follow_world(&mut self, home: InstanceId, limit: usize) {
        self.world_logs.insert(home, WorldLog { limit, ..WorldLog::default() });
    }

    /// The change log of `home`'s world, while it is followed.
    pub fn world_log(&mut self, home: InstanceId) -> Option<&mut WorldLog> {
        self.world_logs.get_mut(&home)
    }

    /// Logs a change of `id` in the world of `home`, if it is followed: a write, or an attach, a detach, a replacement or a class.
    fn log_world(&mut self, home: Option<InstanceId>, id: InstanceId, written: bool) {
        let Some(log) = home.and_then(|home| self.world_logs.get_mut(&home)) else { return };
        if log.overflowed {
            return;
        }
        // (each instance once in each list until the index takes the log)
        if (written && log.written_logged.contains(&id)) || (!written && log.logged.contains(&id)) {
            return;
        }
        if log.moved.len() + log.written.len() >= log.limit {
            log.overflowed = true;
            log.moved = Vec::new();
            log.written = Vec::new();
            log.logged = FxHashSet::default();
            log.written_logged = FxHashSet::default();
        } else if written {
            log.written_logged.insert(id);
            log.written.push(id);
        } else {
            log.logged.insert(id);
            log.moved.push(id);
        }
    }

    /// Logs `id` and the parts below it ([`Dom::keep_logged_classes`]), down to the homes in it, leaving or changing in the world
    /// of `home` (if it is followed): what is below a home is in that home's world, so it is passed over unless `through` (`id`
    /// itself becoming or stopping being one).
    fn log_world_leaving(&mut self, home: Option<InstanceId>, id: InstanceId, through: bool) {
        if !home.is_some_and(|home| self.world_logs.contains_key(&home)) {
            return;
        }
        self.log_world(home, id, false);
        let mut walk = std::mem::take(&mut self.world_walk);
        walk.clear();
        if through || !self.is_home(id) {
            self.at(id).children.copy_to(&mut walk);
        }
        while let Some(node) = walk.pop() {
            let (class, home_class) = (self.at(node).class, self.is_home(node));
            if self.logged_classes.get(class as usize).copied().unwrap_or(false) {
                self.log_world(home, node, false);
            }
            if !home_class {
                self.at(node).children.copy_to(&mut walk);
            }
        }
        self.world_walk = walk;
    }

    /// Logs a write of an indexed instance for the spatial index, once until it takes the log ([`Dom::bump_version`] does this
    /// for every write through it).
    pub fn note_written(&mut self, id: InstanceId) {
        let Some(slot) = self.slots.get_mut(id.index as usize) else { return };
        let Some(instance) = slot.instance.as_mut().filter(|_| slot.generation == id.generation) else { return };
        if instance.flags & (flags::INDEXED | flags::LOGGED) != flags::INDEXED {
            return;
        }
        if let Some(watch) = &mut self.watch
            && watch.room()
        {
            instance.flags |= flags::LOGGED;
            watch.written.push(id);
        }
    }

    /// Whether `ancestor` is a (strict) ancestor of `id`.
    pub fn is_descendant_of(&self, id: InstanceId, ancestor: InstanceId) -> bool {
        self.is_descendant_of_steps(id, ancestor).0
    }

    /// [`Dom::is_descendant_of`], and how many ancestors it looked at.
    pub fn is_descendant_of_steps(&self, id: InstanceId, ancestor: InstanceId) -> (bool, usize) {
        let mut steps = 0;
        let mut current = self.at(id).parent;
        while let Some(node) = current {
            steps += 1;
            if node == ancestor {
                return (true, steps);
            }
            current = self.at(node).parent;
        }
        (false, steps)
    }

    /// Descendants in preorder (GetDescendants' order). The walks of the tree keep their own stack and never recurse: a script
    /// can build a chain of instances deeper than any thread's stack holds frames for.
    pub fn descendants(&self, id: InstanceId) -> Vec<InstanceId> {
        self.descendants_where(id, |_| true)
    }

    /// The descendants of `id` for which `keep` holds, in [`Dom::descendants`]' order, without
    /// listing the others.
    pub fn descendants_where(&self, id: InstanceId, keep: impl Fn(&Instance) -> bool) -> Vec<InstanceId> {
        let mut result = Vec::new();
        let mut stack: Vec<InstanceId> = self.at(id).children.iter().rev().copied().collect();
        while let Some(node) = stack.pop() {
            let instance = self.at(node);
            if keep(instance) {
                result.push(node);
            }
            stack.extend(instance.children.iter().rev());
        }
        result
    }

    /// Root plus descendants breadth-first (the order FireDescendantEvents fires in).
    pub fn subtree_breadth_first(&self, root: InstanceId) -> Vec<InstanceId> {
        let mut list = vec![root];
        let mut index = 0;
        while index < list.len() {
            let node = list[index];
            self.at(node).children.copy_to(&mut list);
            index += 1;
        }
        list
    }

    /// The subtree with every instance after its children (children in order): FireAncestryChanged's order.
    pub fn subtree_post_order(&self, root: InstanceId) -> Vec<InstanceId> {
        // (the stack order, each instance before its children and the last child first, read backwards)
        let mut result = self.subtree_stack_order(root);
        result.reverse();
        result
    }

    /// Root plus descendants in an old FireAncestryChanged's order (a stack: last child first).
    pub fn subtree_stack_order(&self, root: InstanceId) -> Vec<InstanceId> {
        let mut result = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            result.push(node);
            self.at(node).children.copy_to(&mut stack);
        }
        result
    }

    /// GetFullName: names from below the service provider at the root (the DataModel, UserSettings) down to `id`, dot
    /// separated; a parentless tree of other objects keeps its root's name.
    pub fn full_name(&self, reflection: &Reflection, id: InstanceId) -> String {
        self.full_name_counted(reflection, id).0
    }

    /// [`Dom::full_name`], and how many names it joined (what a client's GetFullName walks: cost.rs full_name).
    pub fn full_name_counted(&self, reflection: &Reflection, id: InstanceId) -> (String, usize) {
        let provider = reflection.class_id("ServiceProvider");
        let is_provider = |node: InstanceId| provider.is_some_and(|class| reflection.class(self.at(node).class).is_a(class));
        // a service provider's own full name is its Name (`game:GetFullName() == game.Name` in a playing client, Studio
        // 0.744: tools/parity/probes/instances.luau); below one, paths start at its child (Studio 0.748: Workspace.Terrain,
        // UserSettings():GetService("UserGameSettings") is "GameSettings", settings().Rendering is "Rendering", while a
        // parentless Folder's Part is "Folder.Part")
        if is_provider(id) {
            return (self.at(id).name.to_str_lossy().into_owned(), 1);
        }
        let mut parts: Vec<&Str> = Vec::new();
        let mut current = Some(id);
        while let Some(node) = current {
            let instance = self.at(node);
            if is_provider(node) {
                break;
            }
            parts.push(&instance.name);
            current = instance.parent;
        }
        let mut result = String::new();
        for (index, part) in parts.iter().rev().enumerate() {
            if index > 0 {
                result.push('.');
            }
            result.push_str(&part.to_str_lossy());
        }
        (result, parts.len())
    }

    /// First child named `name`.
    pub fn find_child(&self, parent: InstanceId, name: &[u8]) -> Option<InstanceId> {
        self.at(parent)
            .children
            .iter()
            .copied()
            .find(|&child| self.at(child).name.as_bytes() == name)
    }

    pub fn find_child_of_class(&self, parent: InstanceId, class: ClassId) -> Option<InstanceId> {
        self.at(parent).children.iter().copied().find(|&child| self.at(child).class == class)
    }

    /// Sets the effective name after the Name property changed.
    pub fn refresh_name(&mut self, reflection: &Reflection, id: InstanceId) {
        let instance = self.at(id);
        let name = match instance.stored(K::Name as NameId) {
            Some(Value::String(name)) => name.clone(),
            Some(Value::Nil) => Str::default(),
            Some(_) | None => match self.defaults.get(instance.class, K::Name as NameId) {
                Some(Value::String(name)) => name.clone(),
                _ => Str::from(&*reflection.class(instance.class).name),
            },
        };
        self.at_mut(id).name = name;
        self.name_version = self.name_version.wrapping_add(1);
    }

    /// Stores `value` as RawSet does with one property lookup: (whether RawGet's value changed,
    /// the previous RawGet value when it did (nil when it was unset without a default); nil
    /// otherwise).
    pub fn exchange(&mut self, id: InstanceId, name: NameId, value: Value) -> (bool, Value) {
        let slot = &mut self.slots[id.index as usize];
        assert!(slot.generation == id.generation, "stale instance id");
        let instance = slot.instance.as_mut().expect("stale instance id");
        if value.references_instance() {
            instance.flags |= flags::HOLDS_REFS;
        }
        let props = instance.props.own_mut();
        match props.binary_search_by_key(&name, |(n, _)| *n) {
            Ok(index) => {
                let stored = &mut props[index].1;
                // (a stored nil reads as nil: RawGet does not fall back to the default)
                let same = match &*stored {
                    Value::Nil => value.is_nil(),
                    old => old.same(&value),
                };
                let old = std::mem::replace(stored, value);
                (!same, if same { Value::Nil } else { old })
            }
            Err(index) => {
                let default = self.defaults.get(instance.class, name);
                let same = match default {
                    Some(old) => old.same(&value),
                    None => value.is_nil(),
                };
                let old = if same { Value::Nil } else { default.cloned().unwrap_or(Value::Nil) };
                props.insert(index, (name, value));
                (!same, old)
            }
        }
    }

    pub fn bump_version(&mut self, id: InstanceId) {
        let instance = self.at_mut(id);
        instance.version = instance.version.wrapping_add(1);
        let (indexed, class) = (instance.flags & flags::INDEXED != 0, instance.class);
        self.write_version = self.write_version.wrapping_add(1);
        if indexed {
            self.note_written(id);
        }
        // (a followed world's log keeps its parts' writes alone, as the Workspace's keeps those of the parts it indexes: its
        // index reads nothing else)
        if !self.world_logs.is_empty() && self.logged_classes.get(class as usize).copied().unwrap_or(false) {
            let home = self.home(id);
            self.log_world(home, id, true);
        }
    }

    /// [`Dom::bump_version`] for a write of the property `name`: one no spatial index reads ([`Dom::keep_unread`], a part's
    /// collision group) leaves the instance out of the Workspace's log and its world's, whichever of their indexes was made, so a
    /// collision-group change does not have an index read every part whose id it changed again.
    pub fn bump_version_of(&mut self, id: InstanceId, name: NameId) {
        if self.unread.contains(&name) {
            let instance = self.at_mut(id);
            instance.version = instance.version.wrapping_add(1);
            self.write_version = self.write_version.wrapping_add(1);
        } else {
            self.bump_version(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every instance holds the rarely set parts (a script's source, tags, callbacks) as one pointer
    /// each, and its properties and attributes in 24 bytes: 176 bytes an instance (it was 248; 168 before each child kept its
    /// place in its parent's list: Children).
    #[test]
    fn an_instance_is_176_bytes() {
        assert_eq!(std::mem::size_of::<Instance>(), 176);
    }

    #[test]
    fn tree_queries_and_defaults() {
        let reflection = Reflection::shared().unwrap();
        let mut dom = Dom::new();
        let frame_class = reflection.class_id("Frame").unwrap();
        let root = dom.create(&reflection, reflection.class_id("Folder").unwrap());
        let a = dom.create(&reflection, frame_class);
        let b = dom.create(&reflection, frame_class);
        let c = dom.create(&reflection, frame_class);
        assert_eq!(dom.at(a).serial, 2);
        dom.attach(a, root);
        dom.attach(b, root);
        dom.attach(c, a);
        assert_eq!(dom.descendants(root), vec![a, c, b]);
        assert_eq!(dom.subtree_breadth_first(root), vec![root, a, b, c]);
        assert_eq!(dom.subtree_stack_order(root), vec![root, b, a, c]);
        assert_eq!(dom.subtree_post_order(root), vec![c, a, b, root]);
        assert!(dom.is_descendant_of(c, root) && !dom.is_descendant_of(root, c));
        assert_eq!(dom.full_name(&reflection, c), "Folder.Frame.Frame");
        dom.detach(a);
        assert_eq!(dom.children_of(root).to_vec(), [b]);
        assert_eq!(dom.parent_of(a), None);
        // Defaults: the dump's Frame defaults
        let visible = dom.raw_get(a, K::Visible as NameId).unwrap();
        assert_eq!(visible.as_bool(), Some(true));
        assert_eq!(dom.number(a, K::ZIndex as NameId), Some(1.0));
        dom.at_mut(a).store(K::Visible as NameId, Value::Nil);
        assert!(dom.raw_get(a, K::Visible as NameId).is_none());
        // BindableFunction's recorded Name default differs from its class name
        let function = dom.create(&reflection, reflection.class_id("BindableFunction").unwrap());
        assert_eq!(dom.at(function).name.as_bytes(), b"Function");
    }

    /// The homes the dom keeps (the nearest WorldRoot or JointsService ancestor) are each instance's as a walk up the tree finds
    /// it, after every kind of change the tree takes: instances made under others, moved (`reparent`, as set_parent moves them,
    /// and `attach` / `detach`), moved into and out of WorldModels and JointsService and between chains 2,000 deep, put in
    /// another's place (`replace_child` after the children were handed over, as a project mount does), freed and made again in
    /// the same slot, and given a WorldRoot's class or a Folder's where they stand (`set_class`, a placeholder revealed); homes
    /// kept from a tree that already existed too. What is above each instance's home (the outermost home and how many homes:
    /// [`Dom::homes_above`]) is what the walk finds as well, through chains of WorldModels 200 deep moved into and out of the
    /// Workspace and each other. Whether an instance is below one of them, read from the homes, is what the walk says too, and
    /// each change of a weld's home is in the log of the classes logged ([`Dom::log_homes`]).
    #[test]
    fn homes_are_what_a_walk_up_the_tree_finds() {
        let reflection = Reflection::shared().unwrap();
        let class = |name: &str| reflection.class_id(name).unwrap();
        let (world_root, joints_service) = (class("WorldRoot"), class("JointsService"));
        let homes: Vec<bool> = reflection.classes.iter().enumerate().map(|(id, info)| info.is_a(world_root) || id == joints_service as usize).collect();
        let is_home = |dom: &Dom, id: InstanceId| homes[dom.at(id).class as usize];
        let walked = |dom: &Dom, id: InstanceId| {
            let mut current = dom.at(id).parent;
            while let Some(node) = current {
                if is_home(dom, node) {
                    return Some(node);
                }
                current = dom.at(node).parent;
            }
            None
        };
        let walked_above = |dom: &Dom, id: InstanceId| {
            let (mut above, mut current) = (Above::default(), dom.at(id).parent);
            while let Some(node) = current {
                if is_home(dom, node) {
                    above = Above { top: Some(node), homes: above.homes + 1 };
                }
                current = dom.at(node).parent;
            }
            above
        };
        let mut dom = Dom::new();
        let game = dom.create(&reflection, class("DataModel"));
        let workspace = dom.create(&reflection, class("Workspace"));
        dom.attach(workspace, game);
        let service = dom.create(&reflection, class("JointsService"));
        dom.attach(service, game);
        let storage = dom.create(&reflection, class("ReplicatedStorage"));
        dom.attach(storage, game);
        let mut live = vec![game, workspace, service, storage];
        // a tree made before the homes were kept
        let early = dom.create(&reflection, class("Part"));
        dom.attach(early, workspace);
        live.push(early);
        dom.keep_homes(homes.clone());
        let check = |dom: &Dom, live: &[InstanceId], step: &str| {
            for &id in live {
                assert_eq!(dom.home(id), walked(dom, id), "{step}: the home of {}", dom.full_name(&reflection, id));
                assert_eq!(dom.homes_above(id), walked_above(dom, id), "{step}: what is above {}", dom.full_name(&reflection, id));
            }
        };
        check(&dom, &live, "kept from the start");
        // each weld whose home changed is in the log, whatever changed it: one missing from it since the last look has the home
        // it had then (a new one none)
        let weld = class("Weld");
        dom.log_homes((0..reflection.classes.len()).map(|id| id == weld as usize).collect());
        let mut seen: std::collections::HashMap<InstanceId, Option<InstanceId>> = std::collections::HashMap::new();
        let mut check_log = |dom: &mut Dom, live: &[InstanceId], step: &str| {
            let logged: std::collections::HashSet<InstanceId> = dom.take_home_log().expect("the log kept every change").into_iter().collect();
            for &id in live.iter().filter(|&&id| dom.at(id).class == weld) {
                if !logged.contains(&id) {
                    assert_eq!(seen.get(&id).copied().flatten(), dom.home(id), "{step}: {} changed home unlogged", dom.full_name(&reflection, id));
                }
                seen.insert(id, dom.home(id));
            }
        };
        // whether each is below the Workspace, JointsService or a WorldModel, read from the homes (is_descendant_of_home), is what
        // the walk up the tree says; a stale ancestor is below nothing
        // (and below the WorldModels of the nested chains, at their top, in their middle and at their bottom)
        let check_below = |dom: &Dom, live: &[InstanceId], worlds: &[InstanceId], stale: Option<InstanceId>, step: &str| {
            let ancestors: Vec<InstanceId> = std::iter::once(workspace)
                .chain(live.iter().copied().filter(|&id| id != workspace && is_home(dom, id)).take(3))
                .chain(worlds.iter().copied().filter(|&id| dom.get(id).is_some() && is_home(dom, id)))
                .chain(stale)
                .collect();
            for &id in live {
                for &ancestor in &ancestors {
                    assert_eq!(dom.is_descendant_of_home(id, ancestor), dom.is_descendant_of(id, ancestor), "{step}: whether {} is below {ancestor:?}", dom.full_name(&reflection, id));
                }
            }
        };
        check_below(&dom, &live, &[], None, "kept from the start");
        // two chains 2,000 deep, one in the Workspace, one in storage
        let deep = |dom: &mut Dom, live: &mut Vec<InstanceId>, top: InstanceId| {
            let mut at = top;
            for _ in 0..2000 {
                let folder = dom.create(&reflection, class("Folder"));
                dom.attach(folder, at);
                live.push(folder);
                at = folder;
            }
            at
        };
        let deep_in_world = deep(&mut dom, &mut live, workspace);
        let deep_in_storage = deep(&mut dom, &mut live, storage);
        // and two chains of WorldModels nested 200 deep, one in the Workspace, one in storage
        let nested = |dom: &mut Dom, live: &mut Vec<InstanceId>, top: InstanceId| {
            let mut chain = Vec::new();
            let mut at = top;
            for _ in 0..200 {
                let world = dom.create(&reflection, class("WorldModel"));
                dom.attach(world, at);
                live.push(world);
                chain.push(world);
                at = world;
            }
            chain
        };
        let nested_in_world = nested(&mut dom, &mut live, workspace);
        let nested_in_storage = nested(&mut dom, &mut live, storage);
        let worlds = [nested_in_world[0], nested_in_world[100], nested_in_world[199], nested_in_storage[0], nested_in_storage[100], nested_in_storage[199]];
        let bottoms = [deep_in_world, deep_in_storage, nested_in_world[199], nested_in_storage[199]];
        check(&dom, &live, "the chains");
        check_below(&dom, &live, &worlds, None, "the chains");
        assert_eq!(dom.homes_above(nested_in_world[199]), Above { top: Some(workspace), homes: 200 });
        assert_eq!(dom.homes_above(nested_in_storage[199]), Above { top: Some(nested_in_storage[0]), homes: 199 });
        let mut stale = None;
        let kinds = ["Folder", "Part", "Model", "WorldModel", "Weld", "Folder", "Part"];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        for round in 0..3000 {
            let pick = |next: &mut dyn FnMut(usize) -> usize, live: &[InstanceId]| live[next(live.len())];
            match next(8) {
                // made under a live instance (the chains' bottoms often)
                0 | 1 => {
                    let made = dom.create(&reflection, class(kinds[next(kinds.len())]));
                    let parent = match next(3) {
                        0 => bottoms[next(bottoms.len())],
                        1 => bottoms[next(2)],
                        _ => pick(&mut next, &live),
                    };
                    dom.attach(made, parent);
                    live.push(made);
                }
                // moved under another (never into itself), or out of the tree
                2..=4 => {
                    let moved = pick(&mut next, &live[4..]);
                    let target = if next(10) == 0 { None } else { Some(pick(&mut next, &live)) };
                    if target.is_some_and(|target| target == moved || dom.is_descendant_of(target, moved)) {
                        continue;
                    }
                    if next(2) == 0 {
                        dom.reparent(moved, target);
                    } else {
                        dom.detach(moved);
                        if let Some(target) = target {
                            dom.attach(moved, target);
                        }
                    }
                }
                // put in another's place, its children handed over first
                5 => {
                    let old = pick(&mut next, &live[4..]);
                    let Some(parent) = dom.at(old).parent.filter(|_| !bottoms.contains(&old)) else { continue };
                    let new = dom.create(&reflection, class(kinds[next(kinds.len())]));
                    dom.hand_children(old, new);
                    dom.replace_child(parent, old, new);
                    live.push(new);
                }
                // a leaf taken out and freed (its slot is made again)
                6 => {
                    let index = 5 + next(live.len() - 5);
                    let leaf = live[index];
                    if !dom.at(leaf).children.is_empty() || bottoms.contains(&leaf) {
                        continue;
                    }
                    dom.detach(leaf);
                    dom.free(leaf);
                    live.swap_remove(index);
                    stale = Some(leaf);
                }
                // given another class where it stands
                _ => {
                    let id = pick(&mut next, &live[5..]);
                    let to = if is_home(&dom, id) { "Folder" } else { "WorldModel" };
                    dom.set_class(&reflection, id, class(to));
                }
            }
            if round % 100 == 0 {
                check(&dom, &live, &format!("round {round}"));
            }
            if round % 20 == 0 {
                check_log(&mut dom, &live, &format!("round {round}"));
            }
            if round % 500 == 0 {
                check_below(&dom, &live, &worlds, stale, &format!("round {round}"));
            }
        }
        check(&dom, &live, "the end");
        check_log(&mut dom, &live, "the end");
        check_below(&dom, &live, &worlds, stale, "the end");
        // a JointsService between: its own home is none, those below it have it
        let weld = dom.create(&reflection, class("Weld"));
        dom.attach(weld, service);
        assert_eq!((dom.home(weld), dom.home(service)), (Some(service), None));
    }

    /// Each instance's children are in the order a client keeps them in (GetChildren's) and each child at its place in its parent's
    /// list, after 20,000 random makes under four parents, removals from the start, the end and anywhere (`detach`, and `reparent`
    /// to one of the four), replacements in place (`hand_children`, then `replace_child`), frees, re-attaches, lists put in another
    /// order (`reorder_children`), and a parent put in another's place with its whole list. A model list changed by a client's own
    /// rules (Studio 0.747: out of a list of more than 20 its last child takes the removed one's place, out of a shorter one the gap
    /// closes, a last child is dropped) holds the same children in the same order, and each removal's count of the places a
    /// client's search passes is where the child was (none for the last).
    #[test]
    fn children_keep_a_clients_order_and_their_places_as_they_come_and_go() {
        let reflection = Reflection::shared().unwrap();
        let folder = reflection.class_id("Folder").unwrap();
        let mut dom = Dom::new();
        let root = dom.create(&reflection, folder);
        // (by parent: the order a client keeps them in)
        let mut parents = Vec::new();
        let mut order: FxHashMap<InstanceId, Vec<InstanceId>> = FxHashMap::default();
        for _ in 0..4 {
            let parent = dom.create(&reflection, folder);
            dom.attach(parent, root);
            parents.push(parent);
            order.insert(parent, Vec::new());
        }
        // a client's removal: where the child was in its order (none for the last), its order changed as the client changes it
        let client_take = |list: &mut Vec<InstanceId>, child: InstanceId| {
            let at = list.iter().position(|&id| id == child).expect("in the client's order");
            if at + 1 == list.len() {
                list.pop();
                0
            } else if list.len() <= 20 {
                list.remove(at);
                at
            } else {
                list.swap_remove(at);
                at
            }
        };
        let check = |dom: &Dom, order: &FxHashMap<InstanceId, Vec<InstanceId>>, parents: &[InstanceId], step: &str| {
            for &parent in parents {
                let children = &dom.at(parent).children;
                let listed = &order[&parent];
                assert_eq!(&children.to_vec(), listed, "{step}: the children in a client's order");
                assert!(children.iter().rev().eq(listed.iter().rev()), "{step}: and backwards");
                assert_eq!(children.slice(), &listed[..], "{step}: as a slice");
                assert_eq!((children.len(), children.is_empty(), children.first(), children.last()), (listed.len(), listed.is_empty(), listed.first(), listed.last()), "{step}: the count and ends");
                for (place, &child) in children.iter().enumerate() {
                    assert_eq!((dom.sibling_place(child) as usize, dom.at(child).parent), (place, Some(parent)), "{step}: a child at its place");
                }
            }
        };
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        let mut loose = Vec::new();
        let (mut swapped, mut closed, mut lasts, mut reordered) = (0, 0, 0, 0);
        for round in 0..20_000 {
            let parent = parents[next(parents.len())];
            let len = order[&parent].len();
            // (the first list grows long; the others stay about 20 long, where a client's way of taking one out changes)
            let op = if parent != parents[0] && len >= 24 { 4 } else { next(10) };
            match op {
                // made under one (the first gets the most)
                0..=3 => {
                    let parent = if next(3) == 0 { parents[0] } else { parent };
                    let made = dom.create(&reflection, folder);
                    dom.attach(made, parent);
                    order.get_mut(&parent).unwrap().push(made);
                }
                // one taken out: the first, the last or any; out of the tree, or to one of the four (itself included: to its end)
                4..=6 if len > 0 => {
                    let at = match next(3) {
                        0 => 0,
                        1 => len - 1,
                        _ => next(len),
                    };
                    let child = order[&parent][at];
                    let to = if next(2) == 0 { None } else { Some(parents[next(parents.len())]) };
                    let searched = if to.is_none() && next(2) == 0 { dom.detach(child) } else { dom.reparent(child, to) };
                    let list = order.get_mut(&parent).unwrap();
                    let (before, last) = (list.len(), list.last() == Some(&child));
                    assert_eq!(searched, client_take(list, child), "round {round}: the places a client's search passes");
                    match () {
                        _ if last => lasts += 1,
                        _ if before <= 20 => closed += 1,
                        _ => swapped += 1,
                    }
                    match to {
                        Some(to) => order.get_mut(&to).unwrap().push(child),
                        None => loose.push(child),
                    }
                }
                // one put in another's place, what it holds handed over
                7 if len > 0 => {
                    let at = next(len);
                    let old = order[&parent][at];
                    let new = dom.create(&reflection, folder);
                    dom.hand_children(old, new);
                    dom.replace_child(parent, old, new);
                    order.get_mut(&parent).unwrap()[at] = new;
                    loose.push(old);
                }
                // one out of the tree freed (its slot is made again), or attached again
                8 if !loose.is_empty() => {
                    let id = loose.swap_remove(next(loose.len()));
                    if next(2) == 0 {
                        dom.free(id);
                    } else {
                        dom.attach(id, parent);
                        order.get_mut(&parent).unwrap().push(id);
                    }
                }
                // a list put in another order, a permutation of it
                9 if round % 25 == 5 && len > 1 => {
                    let mut shuffled = order[&parent].clone();
                    for at in (1..shuffled.len()).rev() {
                        shuffled.swap(at, next(at + 1));
                    }
                    dom.reorder_children(parent, &shuffled);
                    reordered += 1;
                    order.insert(parent, shuffled);
                }
                // a parent put in another's place with its whole list, the old one freed
                9 if round % 25 == 0 => {
                    let index = next(parents.len());
                    let old = parents[index];
                    let new = dom.create(&reflection, folder);
                    dom.hand_children(old, new);
                    dom.replace_child(root, old, new);
                    dom.free(old);
                    parents[index] = new;
                    let listed = order.remove(&old).unwrap();
                    order.insert(new, listed);
                }
                _ => {}
            }
            if round % 50 == 0 {
                check(&dom, &order, &parents, &format!("round {round}"));
            }
        }
        check(&dom, &order, &parents, "the end");
        assert!(swapped > 1000 && closed > 300 && lasts > 1000, "each of the client's ways was taken: {swapped} {closed} {lasts}");
        assert!(reordered > 20, "lists put in another order: {reordered}");
        assert!(order[&parents[0]].len() > 1000, "a long list: {}", order[&parents[0]].len());
    }

    /// Taking out a child costs the same host time wherever it is in its parent's list and however long the list is: 30,000 taken
    /// out of a Folder of 1,000,000 Folders (10,000 from the start, each the first then, 10,000 from the middle, 10,000 from the
    /// end, each the last then) take well under 0.2 s (looking for each in the list and closing the gap took 7.1 s), and the rest are
    /// in a client's order (each taken one's place given to the last child).
    #[test]
    fn taking_out_a_child_costs_the_same_host_time_wherever_it_is() {
        let reflection = Reflection::shared().unwrap();
        let folder = reflection.class_id("Folder").unwrap();
        let mut dom = Dom::new();
        let holder = dom.create(&reflection, folder);
        let count = 1_000_000;
        let made: Vec<InstanceId> = (0..count)
            .map(|_| {
                let id = dom.create(&reflection, folder);
                dom.attach(id, holder);
                id
            })
            .collect();
        // (the first then: whatever holds the first place when it is taken)
        let mut model: Vec<usize> = (0..count).collect();
        let mut at: Vec<usize> = (0..count).collect();
        let mut taken: Vec<usize> = Vec::with_capacity(30_000);
        let mut take = |model: &mut Vec<usize>, index: usize| {
            let place = at[index];
            model.swap_remove(place);
            if let Some(&moved) = model.get(place) {
                at[moved] = place;
            }
        };
        for _ in 0..10_000 {
            let first = model[0];
            taken.push(first);
            take(&mut model, first);
        }
        for k in 0..10_000 {
            let index = if k % 2 == 0 { count / 2 + k / 2 } else { count / 2 - 1 - k / 2 };
            taken.push(index);
            take(&mut model, index);
        }
        for _ in 0..10_000 {
            let last = *model.last().unwrap();
            taken.push(last);
            take(&mut model, last);
        }
        let started = std::time::Instant::now();
        for &index in &taken {
            dom.detach(made[index]);
        }
        let took = started.elapsed().as_secs_f64();
        assert!(took < 0.2, "30,000 children taken out of 1,000,000 in {took:.3} s");
        let rest: Vec<InstanceId> = model.iter().map(|&index| made[index]).collect();
        assert!(dom.children_of(holder).to_vec() == rest, "the rest in a client's order");
    }

    /// A class change that renames an instance (its name was the old class's default) moves the names' version, which a child
    /// lookup's cached answer is stamped with; one that keeps its name (stored, or the same default) does not
    #[test]
    fn a_class_change_that_renames_moves_the_names_version() {
        let reflection = Reflection::shared().unwrap();
        let mut dom = Dom::new();
        let (folder, frame) = (reflection.class_id("Folder").unwrap(), reflection.class_id("Frame").unwrap());
        let unnamed = dom.create(&reflection, folder);
        let before = dom.name_version;
        dom.set_class(&reflection, unnamed, frame);
        assert_eq!(dom.at(unnamed).name.as_bytes(), b"Frame");
        assert_ne!(dom.name_version, before, "renamed from Folder to Frame");
        let named = dom.create(&reflection, folder);
        dom.at_mut(named).store(K::Name as NameId, Value::String(Str::from("Kept")));
        dom.refresh_name(&reflection, named);
        let before = dom.name_version;
        dom.set_class(&reflection, named, frame);
        assert_eq!((dom.at(named).name.as_bytes(), dom.name_version), (&b"Kept"[..], before), "a stored name stays");
        dom.set_class(&reflection, unnamed, frame);
        assert_eq!(dom.name_version, before, "the same class: the same name");
    }
}
