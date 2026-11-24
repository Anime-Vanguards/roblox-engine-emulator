#![allow(clippy::missing_panics_doc)]

use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    fmt,
    hash::{Hash, Hasher},
};

#[cfg(feature = "mlua")]
use mlua::prelude::*;

use rbx_dom_weak::{
    Instance as DomInstance, InstanceBuilder as DomInstanceBuilder, Ustr, WeakDom,
    types::{Attributes as DomAttributes, Ref as DomRef, Variant as DomValue},
    ustr,
};

#[cfg(feature = "mlua")]
use lune_utils::TableBuilder;

use crate::shared::instance::class_is_a;

#[cfg(feature = "mlua")]
use crate::{exports::LuaExportsTable, shared::instance::class_exists};

#[cfg(feature = "mlua")]
pub(crate) mod base;
#[cfg(feature = "mlua")]
pub(crate) mod data_model;
#[cfg(feature = "mlua")]
pub(crate) mod terrain;
#[cfg(feature = "mlua")]
pub(crate) mod workspace;

#[cfg(feature = "mlua")]
pub mod registry;

const PROPERTY_NAME_ATTRIBUTES: &str = "Attributes";
const PROPERTY_NAME_TAGS: &str = "Tags";

thread_local! {
    static INTERNAL_DOM: RefCell<WeakDom> = RefCell::new(WeakDom::new(DomInstanceBuilder::new("ROOT")));
    static INSTANCE_IDS: RefCell<(u32, std::collections::HashMap<DomRef, u32>)> = RefCell::new((0, std::collections::HashMap::new()));
}

fn with_dom<R>(f: impl FnOnce(&WeakDom) -> R) -> R {
    INTERNAL_DOM.with(|cell| f(&cell.borrow()))
}

fn with_dom_mut<R>(f: impl FnOnce(&mut WeakDom) -> R) -> R {
    INTERNAL_DOM.with(|cell| f(&mut cell.borrow_mut()))
}

fn instance_id_for_ref(dom_ref: DomRef) -> i64 {
    INSTANCE_IDS.with(|cell| {
        let mut state = cell.borrow_mut();
        if let Some(&id) = state.1.get(&dom_ref) {
            return i64::from(id);
        }
        state.0 = state.0.wrapping_add(1).max(1);
        let id = state.0;
        state.1.insert(dom_ref, id);
        i64::from(id)
    })
}

/// Resets this thread's internal DOM, freeing all instances allocated during a completed VM run.
pub fn reset_internal_dom() {
    with_dom_mut(|dom| *dom = WeakDom::new(DomInstanceBuilder::new("ROOT")));
    INSTANCE_IDS.with(|cell| {
        let mut state = cell.borrow_mut();
        state.0 = 0;
        state.1.clear();
    });
}

#[derive(Debug, Clone, Copy)]
pub struct Instance {
    pub(crate) dom_ref: DomRef,
    pub(crate) class_name: Ustr,
}

impl Instance {
    /**
        Creates a new `Instance` from an existing dom object ref.

        Panics if the instance does not exist in the internal dom,
        or if the given dom object ref points to the internal dom root.

        **WARNING:** Creating a new instance requires locking the internal dom,
        any existing lock must first be released to prevent any deadlocking.
    */
    #[must_use]
    pub fn new(dom_ref: DomRef) -> Self {
        Self::new_opt(dom_ref).expect("Failed to find instance in document")
    }

    /**
        Creates a new `Instance` from a dom object ref, if the instance exists.

        Panics if the given dom object ref points to the internal dom root.

        **WARNING:** Creating a new instance requires locking the internal dom,
        any existing lock must first be released to prevent any deadlocking.
    */
    #[must_use]
    pub fn new_opt(dom_ref: DomRef) -> Option<Self> {
        with_dom(|dom| {
            if let Some(instance) = dom.get_by_ref(dom_ref) {
                assert!(
                    !(instance.referent() == dom.root_ref()),
                    "Instances can not be created from dom roots"
                );

                Some(Self {
                    dom_ref,
                    class_name: instance.class,
                })
            } else {
                None
            }
        })
    }

    /**
        Creates a new orphaned `Instance` with a given class name.

        An orphaned instance is an instance at the root of Lune's internal weak dom.

        **WARNING:** Creating a new instance requires locking the internal dom,
        any existing lock must first be released to prevent any deadlocking.
    */
    #[must_use]
    pub fn new_orphaned(class_name: impl AsRef<str>) -> Self {
        let class_name = class_name.as_ref();
        let dom_ref = with_dom_mut(|dom| {
            let instance = DomInstanceBuilder::new(class_name);
            let dom_root = dom.root_ref();
            dom.insert(dom_root, instance)
        });

        Self {
            dom_ref,
            class_name: ustr(class_name),
        }
    }

    /**
        Creates a new orphaned `Instance` by transferring
        it from an external weak dom to the internal one.

        An orphaned instance is an instance at the root of Lune's internal weak dom.

        Panics if the given dom ref is the root dom ref of the external weak dom.
    */
    #[must_use]
    pub fn from_external_dom(external_dom: &mut WeakDom, external_dom_ref: DomRef) -> Self {
        with_dom_mut(|dom| {
            let dom_root = dom.root_ref();
            external_dom.transfer(external_dom_ref, dom, dom_root);
        });
        Self::new(external_dom_ref)
    }

    /**
        Clones an instance to an external weak dom.

        This will place the instance as a child of the
        root of the weak dom, and return its referent.
    */
    pub fn clone_into_external_dom(self, external_dom: &mut WeakDom) -> DomRef {
        with_dom(|dom| {
            let cloned = dom.clone_into_external(self.dom_ref, external_dom);
            external_dom.transfer_within(cloned, external_dom.root_ref());
            cloned
        })
    }

    /**
        Clones multiple instances to an external weak dom.

        This will place the instances as children of the
        root of the weak dom, and return their referents.
    */
    pub fn clone_multiple_into_external_dom(
        referents: &[DomRef],
        external_dom: &mut WeakDom,
    ) -> Vec<DomRef> {
        with_dom(|dom| {
            let cloned = dom.clone_multiple_into_external(referents, external_dom);

            for referent in &cloned {
                external_dom.transfer_within(*referent, external_dom.root_ref());
            }

            cloned
        })
    }

    /**
        Clones the instance and all of its descendants, and orphans it.

        To then save the new instance it must be re-parented,
        which matches the exact behavior of Roblox's instances.

        ### See Also
        * [`Clone`](https://create.roblox.com/docs/reference/engine/classes/Instance#Clone)
          on the Roblox Developer Hub
    */
    #[must_use]
    pub fn clone_instance(&self) -> Self {
        let new_ref = with_dom_mut(|dom| dom.clone_within(self.dom_ref));

        let new_inst = Self::new(new_ref);
        new_inst.set_parent(None);
        new_inst
    }

    /**
        Destroys the instance, removing it completely
        from the weak dom with no way of recovering it.

        All member methods will throw errors when called from lua and panic
        when called from rust after the instance has been destroyed.

        Returns `true` if destroyed successfully, `false` if already destroyed.

        ### See Also
        * [`Destroy`](https://create.roblox.com/docs/reference/engine/classes/Instance#Destroy)
          on the Roblox Developer Hub
    */
    pub fn destroy(&mut self) -> bool {
        if self.is_destroyed() {
            false
        } else {
            with_dom_mut(|dom| dom.destroy(self.dom_ref));
            true
        }
    }

    fn is_destroyed(&self) -> bool {
        // NOTE: This property can not be cached since instance references
        // other than this one may have destroyed this one, and we don't
        // keep track of all current instance reference structs
        with_dom(|dom| dom.get_by_ref(self.dom_ref).is_none())
    }

    /**
        Destroys all child instances.

        ### See Also
        * [`Instance::Destroy`] for more info about what happens when an instance gets destroyed
        * [`ClearAllChildren`](https://create.roblox.com/docs/reference/engine/classes/Instance#ClearAllChildren)
          on the Roblox Developer Hub
    */
    pub fn clear_all_children(&mut self) {
        with_dom_mut(|dom| {
            let instance = dom
                .get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document");

            let child_refs = instance.children().to_vec();
            for child_ref in child_refs {
                dom.destroy(child_ref);
            }
        });
    }

    /**
        Checks if the instance matches or inherits a given class name.

        ### See Also
        * [`IsA`](https://create.roblox.com/docs/reference/engine/classes/Instance#IsA)
          on the Roblox Developer Hub
    */
    pub fn is_a(&self, class_name: impl AsRef<str>) -> bool {
        class_is_a(self.class_name, class_name).unwrap_or(false)
    }

    /**
        Gets the class name of the instance.

        This will return the correct class name even if the instance has been destroyed.

        ### See Also
        * [`ClassName`](https://create.roblox.com/docs/reference/engine/classes/Instance#ClassName)
          on the Roblox Developer Hub
    */
    #[must_use]
    pub fn get_class_name(&self) -> &str {
        self.class_name.as_str()
    }

    /**
        Gets the name of the instance, if it exists.

        ### See Also
        * [`Name`](https://create.roblox.com/docs/reference/engine/classes/Instance#Name)
          on the Roblox Developer Hub
    */
    pub fn get_name(&self) -> String {
        with_dom(|dom| {
            dom.get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document")
                .name
                .clone()
        })
    }

    /**
        Sets the name of the instance, if it exists.

        ### See Also
        * [`Name`](https://create.roblox.com/docs/reference/engine/classes/Instance#Name)
          on the Roblox Developer Hub
    */
    pub fn set_name(&self, name: impl Into<String>) {
        let name = name.into();
        with_dom_mut(|dom| {
            dom.get_by_ref_mut(self.dom_ref)
                .expect("Failed to find instance in document")
                .name = name;
        });
    }

    /**
        Gets the parent of the instance, if it exists.

        ### See Also
        * [`Parent`](https://create.roblox.com/docs/reference/engine/classes/Instance#Parent)
          on the Roblox Developer Hub
    */
    pub fn get_parent(&self) -> Option<Instance> {
        let parent_ref = with_dom(|dom| {
            let parent_ref = dom.get_by_ref(self.dom_ref)?.parent();
            if parent_ref == dom.root_ref() {
                None
            } else {
                Some(parent_ref)
            }
        })?;
        Some(Self::new(parent_ref))
    }

    /**
        Sets the parent of the instance, if it exists.

        If the provided parent is [`None`] the instance will become orphaned.

        An orphaned instance is an instance at the root of Lune's internal weak dom.

        ### See Also
        * [`Parent`](https://create.roblox.com/docs/reference/engine/classes/Instance#Parent)
          on the Roblox Developer Hub
    */
    pub fn set_parent(&self, parent: Option<Instance>) {
        with_dom_mut(|dom| {
            let parent_ref = parent.map_or_else(|| dom.root_ref(), |parent| parent.dom_ref);
            dom.transfer_within(self.dom_ref, parent_ref);
        });
    }

    /**
        Gets a property for the instance, if it exists.
    */
    pub fn get_property(&self, name: impl AsRef<str>) -> Option<DomValue> {
        let key = ustr(name.as_ref());
        with_dom(|dom| {
            dom.get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document")
                .properties
                .get(&key)
                .cloned()
        })
    }

    /**
        Sets a property for the instance.

        Note that setting a property here will not fail even if the
        property does not actually exist for the instance class.
    */
    pub fn set_property(&self, name: impl AsRef<str>, value: DomValue) {
        let key = ustr(name.as_ref());
        with_dom_mut(|dom| {
            dom.get_by_ref_mut(self.dom_ref)
                .expect("Failed to find instance in document")
                .properties
                .insert(key, value);
        });
    }

    /**
        Gets an attribute for the instance, if it exists.

        ### See Also
        * [`GetAttribute`](https://create.roblox.com/docs/reference/engine/classes/Instance#GetAttribute)
          on the Roblox Developer Hub
    */
    pub fn get_attribute(&self, name: impl AsRef<str>) -> Option<DomValue> {
        with_dom(|dom| {
            let inst = dom
                .get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document");
            if let Some(DomValue::Attributes(attributes)) =
                inst.properties.get(&ustr(PROPERTY_NAME_ATTRIBUTES))
            {
                attributes.get(name.as_ref()).cloned()
            } else {
                None
            }
        })
    }

    /**
        Gets all known attributes for the instance.

        ### See Also
        * [`GetAttributes`](https://create.roblox.com/docs/reference/engine/classes/Instance#GetAttributes)
          on the Roblox Developer Hub
    */
    pub fn get_attributes(&self) -> BTreeMap<String, DomValue> {
        with_dom(|dom| {
            let inst = dom
                .get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document");
            if let Some(DomValue::Attributes(attributes)) =
                inst.properties.get(&ustr(PROPERTY_NAME_ATTRIBUTES))
            {
                attributes.clone().into_iter().collect()
            } else {
                BTreeMap::new()
            }
        })
    }

    /**
        Sets an attribute for the instance.

        ### See Also
        * [`SetAttribute`](https://create.roblox.com/docs/reference/engine/classes/Instance#SetAttribute)
          on the Roblox Developer Hub
    */
    pub fn set_attribute(&self, name: impl AsRef<str>, value: DomValue) {
        with_dom_mut(|dom| {
            let inst = dom
                .get_by_ref_mut(self.dom_ref)
                .expect("Failed to find instance in document");
            // NOTE: Attributes do not support integers, only floats
            let value = match value {
                DomValue::Int32(i) => DomValue::Float32(i as f32),
                DomValue::Int64(i) => DomValue::Float64(i as f64),
                value => value,
            };
            if let Some(DomValue::Attributes(attributes)) =
                inst.properties.get_mut(&ustr(PROPERTY_NAME_ATTRIBUTES))
            {
                attributes.insert(name.as_ref().to_string(), value);
            } else {
                let mut attributes = DomAttributes::new();
                attributes.insert(name.as_ref().to_string(), value);
                inst.properties.insert(
                    ustr(PROPERTY_NAME_ATTRIBUTES),
                    DomValue::Attributes(attributes),
                );
            }
        });
    }

    /**
        Removes an attribute from the instance.

        Note that this does not have an equivalent in the Roblox engine API,
        but separating this from `set_attribute` lets `set_attribute` be more
        ergonomic and not require an `Option<DomValue>` for the value argument.
        The equivalent in the Roblox engine API would be `instance:SetAttribute(name, nil)`.
    */
    pub fn remove_attribute(&self, name: impl AsRef<str>) {
        with_dom_mut(|dom| {
            let inst = dom
                .get_by_ref_mut(self.dom_ref)
                .expect("Failed to find instance in document");
            if let Some(DomValue::Attributes(attributes)) =
                inst.properties.get_mut(&ustr(PROPERTY_NAME_ATTRIBUTES))
            {
                attributes.remove(name.as_ref());
                if attributes.is_empty() {
                    inst.properties.remove(&ustr(PROPERTY_NAME_ATTRIBUTES));
                }
            }
        });
    }

    /**
        Adds a tag to the instance.

        ### See Also
        * [`AddTag`](https://create.roblox.com/docs/reference/engine/classes/CollectionService#AddTag)
          on the Roblox Developer Hub
    */
    pub fn add_tag(&self, name: impl AsRef<str>) {
        with_dom_mut(|dom| {
            let inst = dom
                .get_by_ref_mut(self.dom_ref)
                .expect("Failed to find instance in document");
            if let Some(DomValue::Tags(tags)) = inst.properties.get_mut(&ustr(PROPERTY_NAME_TAGS)) {
                tags.push(name.as_ref());
            } else {
                inst.properties.insert(
                    ustr(PROPERTY_NAME_TAGS),
                    DomValue::Tags(vec![name.as_ref().to_string()].into()),
                );
            }
        });
    }

    /**
        Gets all current tags for the instance.

        ### See Also
        * [`GetTags`](https://create.roblox.com/docs/reference/engine/classes/CollectionService#GetTags)
          on the Roblox Developer Hub
    */
    pub fn get_tags(&self) -> Vec<String> {
        with_dom(|dom| {
            let inst = dom
                .get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document");
            if let Some(DomValue::Tags(tags)) = inst.properties.get(&ustr(PROPERTY_NAME_TAGS)) {
                tags.iter().map(ToString::to_string).collect()
            } else {
                Vec::new()
            }
        })
    }

    /**
        Checks if the instance has a specific tag.

        ### See Also
        * [`HasTag`](https://create.roblox.com/docs/reference/engine/classes/CollectionService#HasTag)
          on the Roblox Developer Hub
    */
    pub fn has_tag(&self, name: impl AsRef<str>) -> bool {
        with_dom(|dom| {
            let inst = dom
                .get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document");
            if let Some(DomValue::Tags(tags)) = inst.properties.get(&ustr(PROPERTY_NAME_TAGS)) {
                let name = name.as_ref();
                tags.iter().any(|tag| tag == name)
            } else {
                false
            }
        })
    }

    /**
        Removes a tag from the instance.

        ### See Also
        * [`RemoveTag`](https://create.roblox.com/docs/reference/engine/classes/CollectionService#RemoveTag)
          on the Roblox Developer Hub
    */
    pub fn remove_tag(&self, name: impl AsRef<str>) {
        with_dom_mut(|dom| {
            let inst = dom
                .get_by_ref_mut(self.dom_ref)
                .expect("Failed to find instance in document");
            if let Some(DomValue::Tags(tags)) = inst.properties.get_mut(&ustr(PROPERTY_NAME_TAGS)) {
                let name = name.as_ref();
                let mut new_tags = tags.iter().map(ToString::to_string).collect::<Vec<_>>();
                new_tags.retain(|tag| tag != name);
                inst.properties
                    .insert(ustr(PROPERTY_NAME_TAGS), DomValue::Tags(new_tags.into()));
            }
        });
    }

    /**
        Gets all of the current children of this `Instance`.

        Note that this is a somewhat expensive operation and that other
        operations using weak dom referents should be preferred if possible.

        ### See Also
        * [`GetChildren`](https://create.roblox.com/docs/reference/engine/classes/Instance#GetChildren)
          on the Roblox Developer Hub
    */
    pub fn get_children(&self) -> Vec<Instance> {
        let children = with_dom(|dom| {
            dom.get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document")
                .children()
                .to_vec()
        });
        children.into_iter().map(Self::new).collect()
    }

    /**
        Gets all of the current descendants of this `Instance` using a breadth-first search.

        Note that this is a somewhat expensive operation and that other
        operations using weak dom referents should be preferred if possible.

        ### See Also
        * [`GetDescendants`](https://create.roblox.com/docs/reference/engine/classes/Instance#GetDescendants)
          on the Roblox Developer Hub
    */
    pub fn get_descendants(&self) -> Vec<Instance> {
        let descendants = with_dom(|dom| {
            let mut descendants = Vec::new();
            let mut queue = VecDeque::from_iter(
                dom.get_by_ref(self.dom_ref)
                    .expect("Failed to find instance in document")
                    .children(),
            );

            while let Some(queue_ref) = queue.pop_front() {
                descendants.push(*queue_ref);
                let queue_inst = dom.get_by_ref(*queue_ref).unwrap();
                for queue_ref_inner in queue_inst.children().iter().rev() {
                    queue.push_back(queue_ref_inner);
                }
            }
            descendants
        });
        descendants.into_iter().map(Self::new).collect()
    }

    /**
        Gets the "full name" of this instance.

        This will be a path composed of instance names from the top-level
        ancestor of this instance down to itself, in the following format:

        `Ancestor.Child.Descendant.Instance`

        ### See Also
        * [`GetFullName`](https://create.roblox.com/docs/reference/engine/classes/Instance#GetFullName)
          on the Roblox Developer Hub
    */
    pub fn get_full_name(&self) -> String {
        with_dom(|dom| {
            let dom_root = dom.root_ref();

            let mut parts = Vec::new();
            let mut instance_ref = self.dom_ref;

            while let Some(instance) = dom.get_by_ref(instance_ref) {
                if instance_ref != dom_root && instance.class != "DataModel" {
                    instance_ref = instance.parent();
                    parts.push(instance.name.clone());
                } else {
                    break;
                }
            }

            parts.reverse();
            parts.join(".")
        })
    }

    /**
        Finds a child of the instance using the given predicate callback.

        ### See Also
        * [`FindFirstChild`](https://create.roblox.com/docs/reference/engine/classes/Instance#FindFirstChild) on the Roblox Developer Hub
        * [`FindFirstChildOfClass`](https://create.roblox.com/docs/reference/engine/classes/Instance#FindFirstChildOfClass) on the Roblox Developer Hub
        * [`FindFirstChildWhichIsA`](https://create.roblox.com/docs/reference/engine/classes/Instance#FindFirstChildWhichIsA) on the Roblox Developer Hub
    */
    pub fn find_child<F>(&self, predicate: F) -> Option<Instance>
    where
        F: Fn(&DomInstance) -> bool,
    {
        let found_ref = with_dom(|dom| {
            let children = dom
                .get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document")
                .children()
                .to_vec();

            children.into_iter().find(|child_ref| {
                if let Some(child_inst) = dom.get_by_ref(*child_ref) {
                    predicate(child_inst)
                } else {
                    false
                }
            })
        });
        found_ref.map(Self::new)
    }

    /**
        Finds an ancestor of the instance using the given predicate callback.

        ### See Also
        * [`FindFirstAncestor`](https://create.roblox.com/docs/reference/engine/classes/Instance#FindFirstAncestor) on the Roblox Developer Hub
        * [`FindFirstAncestorOfClass`](https://create.roblox.com/docs/reference/engine/classes/Instance#FindFirstAncestorOfClass) on the Roblox Developer Hub
        * [`FindFirstAncestorWhichIsA`](https://create.roblox.com/docs/reference/engine/classes/Instance#FindFirstAncestorWhichIsA) on the Roblox Developer Hub
    */
    pub fn find_ancestor<F>(&self, predicate: F) -> Option<Instance>
    where
        F: Fn(&DomInstance) -> bool,
    {
        let found_ref = with_dom(|dom| {
            let mut ancestor_ref = dom
                .get_by_ref(self.dom_ref)
                .expect("Failed to find instance in document")
                .parent();

            while let Some(ancestor) = dom.get_by_ref(ancestor_ref) {
                if predicate(ancestor) {
                    return Some(ancestor_ref);
                }
                ancestor_ref = ancestor.parent();
            }

            None
        });
        found_ref.map(Self::new)
    }

    /**
        Finds a descendant of the instance using the given
        predicate callback and a breadth-first search.

        ### See Also
        * [`FindFirstDescendant`](https://create.roblox.com/docs/reference/engine/classes/Instance#FindFirstDescendant) on the Roblox Developer Hub
    */
    pub fn find_descendant<F>(&self, predicate: F) -> Option<Instance>
    where
        F: Fn(&DomInstance) -> bool,
    {
        let found_ref = with_dom(|dom| {
            let mut queue = VecDeque::from_iter(
                dom.get_by_ref(self.dom_ref)
                    .expect("Failed to find instance in document")
                    .children(),
            );

            while let Some(queue_item) = queue
                .pop_front()
                .and_then(|queue_ref| dom.get_by_ref(*queue_ref))
            {
                if predicate(queue_item) {
                    return Some(queue_item.referent());
                }
                queue.extend(queue_item.children());
            }

            None
        });
        found_ref.map(Self::new)
    }
}

#[cfg(feature = "mlua")]
impl Instance {
    /// Returns the canonical interned Lua userdata for this instance (preserving Lua table-key identity
    /// across `FindFirstChild`, `GetChildren`, `Parent`, `GetDescendants`, and `DomValue::Ref`).
    pub fn into_cached_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        const CACHE_KEY: &str = "__lune_instance_ud_cache";
        let cache: LuaTable = match lua.named_registry_value(CACHE_KEY) {
            Ok(tbl) => tbl,
            Err(_) => {
                let tbl = lua.create_table()?;
                let mt = lua.create_table_with_capacity(0, 1)?;
                mt.raw_set("__mode", "v")?;
                tbl.set_metatable(Some(mt))?;
                lua.set_named_registry_value(CACHE_KEY, tbl.clone())?;
                tbl
            }
        };
        let id = instance_id_for_ref(self.dom_ref);
        if let Ok(existing @ LuaValue::UserData(_)) = cache.raw_get::<LuaValue>(id) {
            return Ok(existing);
        }
        let ud = LuaValue::UserData(lua.create_userdata(self)?);
        cache.raw_set(id, ud.clone())?;
        Ok(ud)
    }

    pub fn opt_into_cached_lua(opt: Option<Self>, lua: &Lua) -> LuaResult<LuaValue> {
        match opt {
            Some(inst) => inst.into_cached_lua(lua),
            None => Ok(LuaValue::Nil),
        }
    }

    pub fn vec_into_cached_lua(items: Vec<Self>, lua: &Lua) -> LuaResult<LuaTable> {
        let tbl = lua.create_table_with_capacity(items.len(), 0)?;
        for (idx, inst) in items.into_iter().enumerate() {
            tbl.raw_set(idx + 1, inst.into_cached_lua(lua)?)?;
        }
        Ok(tbl)
    }
}

#[cfg(feature = "mlua")]
impl LuaExportsTable for Instance {
    const EXPORT_NAME: &'static str = "Instance";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        let instance_new = |lua: &Lua, class_name: String| {
            if class_exists(&class_name) {
                Instance::new_orphaned(class_name).into_cached_lua(lua)
            } else {
                Err(LuaError::RuntimeError(format!(
                    "Failed to create Instance - '{class_name}' is not a valid class name",
                )))
            }
        };

        TableBuilder::new(lua)?
            .with_function("new", instance_new)?
            .build_readonly()
    }
}

/*
    Here we add inheritance-like behavior for instances by creating
    fields that are restricted to specific classnames / base classes

    Note that we should try to be conservative with how many classes
    and methods we support here - we should only implement methods that
    are necessary for modifying the dom and / or having ergonomic access
    to the dom, not try to replicate Roblox engine behavior of instances

    If a user wants to replicate Roblox engine behavior, they can use the
    instance registry, and register properties + methods from the lua side
*/
#[cfg(feature = "mlua")]
impl LuaUserData for Instance {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        data_model::add_fields(fields);
        workspace::add_fields(fields);
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        base::add_methods(methods);
        data_model::add_methods(methods);
        terrain::add_methods(methods);
    }
}

impl Hash for Instance {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dom_ref.hash(state);
    }
}

impl fmt::Display for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            if self.is_destroyed() {
                "<<DESTROYED>>".to_string()
            } else {
                self.get_name()
            }
        )
    }
}

impl PartialEq for Instance {
    fn eq(&self, other: &Self) -> bool {
        self.dom_ref == other.dom_ref
    }
}

impl From<Instance> for DomRef {
    fn from(value: Instance) -> Self {
        value.dom_ref
    }
}
