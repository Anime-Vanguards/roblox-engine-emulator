//! ReflectionService as a game script gets it: GetClass, GetClasses, GetPropertiesOfClass, GetMethodsOfClass and
//! GetEventsOfClass, answered from services/reflection_data.rs (data/reflection.json, a game script's default view, and
//! data/reflection-security.json, what a Security filter adds).
//!
//! Measured in roblox-sim's 0.747 playing client and its server, which answer alike (identity 2; tools/parity/probes/
//! reflection.luau and reflection_members.luau; docs/engine.md "ReflectionService"):
//! - GetClasses lists the classes the filter's Security set may get in name order, each call a new frozen list of new frozen
//!   entries: `Name`, `Superclass` (absent for Object), `Serialized` (true, or absent), `Subclasses` (every subclass in name
//!   order, those no set gets included; absent when there is none), `Display` (Category and, where the class has one,
//!   DeprecationMessage) and `Permits` (New / GetService, each a new SecurityCapabilities value, those the set holds). The
//!   class filter is a dictionary: `Security` (a SecurityCapabilities value; a game script's own when absent), `IsA` (the
//!   class and its subclasses; a name no class has gives none), `ExcludeDisplay` (no Display); other keys are ignored.
//! - the member queries list a class's members, its own first and then each superclass's (a name already listed is not listed
//!   again), each call new frozen tables: a property `Name`, `Owner`, `Type` (EngineType, ScriptType, EnumType, InstanceType:
//!   those it has), `Serialized` (always), `ContentType` (an Enum.AssetType item, where it has one), `Display`, `Permits`
//!   (Read, Write, ReadParallel, WriteParallel); a method `Name`, `Owner`, `Parameters` (each `Name`, `Type` and its
//!   `DefaultValue`, a new value each call), `ReturnType`, `Display` (DeprecationMessage, or empty), `Permits` (Call,
//!   CallParallel); an event `Name`, `Owner`, `Parameters`, `Display`, `Permits` (Listen). The member filter: `Security`,
//!   `ExcludeInherited` (the class's own members only), `ExcludeDisplay`. A name no class has, or a class the set may not
//!   query, answers nil.
//! - casts: the class name is read as a client reads a string parameter (services::strings; nil "Argument 2 missing or nil");
//!   the filter is nil or a dictionary ("Unable to cast double to Dictionary", "... Array to Dictionary"), checked before the
//!   class is looked up; its fields are read raw only for a class there is, Security first (class filter: Security, IsA,
//!   ExcludeDisplay; member filter: Security, ExcludeInherited, ExcludeDisplay): "ReflectionMemberFilter's ExcludeInherited
//!   field is expected to be type bool".
//! - not matched: the order a script's `pairs` walks an entry's keys in (a client's tables are not built as ree's are).

use std::rc::Rc;

use mlua::prelude::*;

use super::reflection_data::{self as data, Caps, Class, Data, Display, Parameter, Permits, Type};
use super::{Behaviors, StringParam, method, strings, values};
use crate::{
    Engine,
    lua::{Args, Flow, raise, typeof_value},
};

/// A value's name in a failed cast ("Unable to cast double to Dictionary"), as the engine names it.
fn cast_name(engine: &Engine, value: &LuaValue) -> String {
    match value {
        LuaValue::Number(_) | LuaValue::Integer(_) => "double".into(),
        LuaValue::Boolean(_) => "bool".into(),
        LuaValue::Table(table) if table.raw_len() > 0 => "Array".into(),
        other => typeof_value(engine, other),
    }
}

/// The filter argument at `index`: nil or a dictionary (measured: a number "Unable to cast double to Dictionary", `{ "BasePart" }` and
/// `{ 1, IsA = "BasePart" }` "Unable to cast Array to Dictionary"), its fields not read yet.
fn dictionary(engine: &Engine, args: &Args, index: usize) -> Result<Option<LuaTable>, Flow> {
    match args.get(index) {
        LuaValue::Nil => Ok(None),
        LuaValue::Table(table) if table.raw_len() == 0 => Ok(Some(table.clone())),
        other => raise(format!("Unable to cast {} to Dictionary", cast_name(engine, other))),
    }
}

/// A filter's Security field: a game script's set when absent, else a SecurityCapabilities value's (prelude CapabilityMask)
fn security(engine: &Engine, table: Option<&LuaTable>, filter: &str) -> Result<Caps, Flow> {
    let Some(table) = table else { return Ok(data::SCRIPT_CAPS) };
    match table.raw_get::<LuaValue>("Security")? {
        LuaValue::Nil => Ok(data::SCRIPT_CAPS),
        value => {
            let mask: LuaValue = engine.prelude().exports.raw_get::<LuaFunction>("CapabilityMask")?.call(value)?;
            match mask {
                LuaValue::Number(mask) => Ok(mask as Caps),
                LuaValue::Integer(mask) => Ok(mask as Caps),
                _ => raise(format!("{filter}'s Security field is expected to be type SecurityCapabilities")),
            }
        }
    }
}

/// A filter's boolean field (false when absent)
fn flag(table: Option<&LuaTable>, filter: &str, key: &str) -> Result<bool, Flow> {
    let Some(table) = table else { return Ok(false) };
    match table.raw_get::<LuaValue>(key)? {
        LuaValue::Nil => Ok(false),
        LuaValue::Boolean(value) => Ok(value),
        _ => raise(format!("{filter}'s {key} field is expected to be type bool")),
    }
}

/// A ReflectionClassFilter, checked (Security, then IsA, then ExcludeDisplay: measured with several fields wrong at once).
struct ClassFilter {
    security: Caps,
    /// `IsA`: the class entries must derive from; `Some(None)` names no class (nothing matches)
    is_a: Option<Option<u32>>,
    display: bool,
}

fn class_filter(engine: &Engine, data: &Data, table: Option<&LuaTable>) -> Result<ClassFilter, Flow> {
    let security = security(engine, table, "ReflectionClassFilter")?;
    let is_a = match table {
        None => None,
        Some(table) => match table.raw_get::<LuaValue>("IsA")? {
            LuaValue::Nil => None,
            LuaValue::String(name) => Some(data.index(&name.to_string_lossy())),
            _ => return raise("ReflectionClassFilter's IsA field is expected to be type string"),
        },
    };
    let display = !flag(table, "ReflectionClassFilter", "ExcludeDisplay")?;
    Ok(ClassFilter { security, is_a, display })
}

/// Whether GetClass and GetClasses give `class` under the set
fn listed(class: &Class, security: Caps) -> bool {
    class.listed && data::holds(security, class.list)
}

fn class_matches(data: &Data, index: u32, filter: &ClassFilter) -> bool {
    listed(&data.classes[index as usize], filter.security)
        && match filter.is_a {
            None => true,
            Some(None) => false,
            Some(Some(ancestor)) => data.is_a(index, ancestor),
        }
}

fn frozen(table: LuaTable) -> LuaTable {
    table.set_readonly(true);
    table
}

/// A Permits table: each action the set holds the capabilities of, a new SecurityCapabilities value (prelude CapabilitiesOfMask)
fn permits_table(lua: &Lua, make: &LuaFunction, permits: &Permits, security: Caps, count: &mut usize) -> LuaResult<LuaTable> {
    let table = lua.create_table_with_capacity(0, permits.len())?;
    for &(action, caps) in permits.iter() {
        if data::holds(security, caps) {
            table.raw_set(action.0, make.call::<LuaValue>(caps as f64)?)?;
            *count += 1;
        }
    }
    Ok(frozen(table))
}

fn display_table(lua: &Lua, display: &Display) -> LuaResult<LuaTable> {
    let table = lua.create_table_with_capacity(0, display.len())?;
    for (key, text) in display.iter() {
        table.raw_set(&**key, &**text)?;
    }
    Ok(frozen(table))
}

fn type_table(lua: &Lua, ty: &Type) -> LuaResult<LuaTable> {
    let table = lua.create_table_with_capacity(0, 4)?;
    for (key, value) in [("EngineType", &ty.engine), ("ScriptType", &ty.script), ("EnumType", &ty.enum_name), ("InstanceType", &ty.instance)] {
        if let Some(value) = value {
            table.raw_set(key, &**value)?;
        }
    }
    Ok(frozen(table))
}

/// A parameter's DefaultValue, a new value each call: what the dump encodes (crate::value::decode_default), a new frozen table
/// for a table, a new RaycastParams or OverlapParams of the defaults, and for a type ree has no value of (a User) a value that
/// is typeof that type and prints the dump's text (prelude OpaqueValue)
fn default_value(lua: &Lua, engine: &Engine, encoded: &serde_json::Value) -> LuaResult<LuaValue> {
    use serde_json::Value as J;
    if let J::Object(map) = encoded {
        let exports = &engine.prelude().exports;
        match map.get("$t").and_then(J::as_str) {
            Some("table") => {
                let table = lua.create_table()?;
                if let Some(J::Object(items)) = map.get("v") {
                    for (key, item) in items {
                        let key: LuaValue = match key.parse::<f64>() {
                            Ok(number) => LuaValue::Number(number),
                            Err(_) => LuaValue::String(lua.create_string(key)?),
                        };
                        table.raw_set(key, default_value(lua, engine, item)?)?;
                    }
                }
                return Ok(LuaValue::Table(frozen(table)));
            }
            Some(kind @ ("RaycastParams" | "OverlapParams")) => {
                let library: LuaTable = exports.raw_get(kind)?;
                return library.raw_get::<LuaFunction>("new")?.call(());
            }
            Some(kind) => {
                if let Some(value) = crate::value::decode_json(&engine.reflection, encoded) {
                    return crate::lua::convert::to_lua(lua, engine, &value);
                }
                let text = map.get("s").and_then(J::as_str).unwrap_or("");
                return exports.raw_get::<LuaFunction>("OpaqueValue")?.call((kind, text));
            }
            None => {}
        }
    }
    match crate::value::decode_json(&engine.reflection, encoded) {
        Some(value) => crate::lua::convert::to_lua(lua, engine, &value),
        None => Ok(LuaValue::Nil),
    }
}

fn parameters_table(lua: &Lua, engine: &Engine, parameters: &[Parameter]) -> LuaResult<LuaTable> {
    let list = lua.create_table_with_capacity(parameters.len(), 0)?;
    for (index, parameter) in parameters.iter().enumerate() {
        let entry = lua.create_table_with_capacity(0, 3)?;
        entry.raw_set("Name", &*parameter.name)?;
        entry.raw_set("Type", type_table(lua, &parameter.ty)?)?;
        if let Some(default) = &parameter.default {
            entry.raw_set("DefaultValue", default_value(lua, engine, default)?)?;
        }
        list.raw_set(index + 1, frozen(entry))?;
    }
    Ok(frozen(list))
}

/// What a member query made, for its price (cost.rs REFLECTED_*)
#[derive(Default)]
struct Made {
    entries: usize,
    parameters: usize,
    permits: usize,
    displays: usize,
    hidden: usize,
}

/// The work clock's price of an answer to a member query of `kind`
fn charge_members(engine: &Engine, kind: Kind, made: &Made) {
    use crate::cost::*;
    let entry = match kind {
        Kind::Properties => REFLECTED_PROPERTY,
        Kind::Methods => REFLECTED_METHOD,
        Kind::Events => REFLECTED_EVENT,
    };
    let seconds = REFLECTED_MEMBER_LIST
        + made.entries as f64 * entry
        + made.parameters as f64 * REFLECTED_PARAMETER
        + made.permits as f64 * REFLECTED_PERMIT
        + made.displays as f64 * REFLECTED_MEMBER_DISPLAY
        + made.hidden as f64 * REFLECTED_PROPERTY_HIDDEN;
    charge_seconds(engine, seconds);
}

fn charge_seconds(engine: &Engine, seconds: f64) {
    engine.state.borrow().sched.charge(seconds);
    crate::boundary::add_cost(seconds);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Properties,
    Methods,
    Events,
}

/// A member query's answer: the class's members (and its superclasses' unless `own`), those the set lists
fn members_list(lua: &Lua, engine: &Engine, data: &Data, index: u32, kind: Kind, security: Caps, own: bool, display: bool) -> LuaResult<(LuaTable, Made)> {
    let make: LuaFunction = engine.prelude().exports.raw_get("CapabilitiesOfMask")?;
    let mut made = Made::default();
    let mut seen: rustc_hash::FxHashSet<&str> = rustc_hash::FxHashSet::default();
    let mut entries: Vec<LuaTable> = Vec::new();
    let mut class = Some(index);
    while let Some(at) = class {
        let owner = &data.classes[at as usize];
        let declared = data.members(at);
        match kind {
            Kind::Properties => {
                for property in declared.properties.iter() {
                    if !data::holds(security, property.visible) {
                        made.hidden += 1;
                        continue;
                    }
                    if !seen.insert(&property.name) {
                        continue;
                    }
                    let entry = lua.create_table_with_capacity(0, 7)?;
                    entry.raw_set("Name", &*property.name)?;
                    entry.raw_set("Owner", &*owner.name)?;
                    entry.raw_set("Type", type_table(lua, &property.ty)?)?;
                    entry.raw_set("Serialized", property.serialized)?;
                    if let Some(content) = &property.content
                        && let Some(item) = engine.reflection.enum_item("AssetType", content)
                    {
                        entry.raw_set("ContentType", crate::lua::enum_item_value(lua, engine, item)?)?;
                    }
                    if display {
                        entry.raw_set("Display", display_table(lua, &property.display)?)?;
                        made.displays += 1;
                    }
                    entry.raw_set("Permits", permits_table(lua, &make, &property.permits, security, &mut made.permits)?)?;
                    entries.push(frozen(entry));
                }
            }
            Kind::Methods => {
                for member in declared.methods.iter() {
                    if !seen.insert(&member.name) {
                        continue;
                    }
                    let entry = lua.create_table_with_capacity(0, 6)?;
                    entry.raw_set("Name", &*member.name)?;
                    entry.raw_set("Owner", &*owner.name)?;
                    entry.raw_set("Parameters", parameters_table(lua, engine, &member.parameters)?)?;
                    made.parameters += member.parameters.len();
                    entry.raw_set("ReturnType", type_table(lua, &member.returns)?)?;
                    if display {
                        entry.raw_set("Display", display_table(lua, &member.display)?)?;
                        made.displays += 1;
                    }
                    entry.raw_set("Permits", permits_table(lua, &make, &member.permits, security, &mut made.permits)?)?;
                    entries.push(frozen(entry));
                }
            }
            Kind::Events => {
                for member in declared.events.iter() {
                    if !seen.insert(&member.name) {
                        continue;
                    }
                    let entry = lua.create_table_with_capacity(0, 5)?;
                    entry.raw_set("Name", &*member.name)?;
                    entry.raw_set("Owner", &*owner.name)?;
                    entry.raw_set("Parameters", parameters_table(lua, engine, &member.parameters)?)?;
                    made.parameters += member.parameters.len();
                    if display {
                        entry.raw_set("Display", display_table(lua, &member.display)?)?;
                        made.displays += 1;
                    }
                    entry.raw_set("Permits", permits_table(lua, &make, &member.permits, security, &mut made.permits)?)?;
                    entries.push(frozen(entry));
                }
            }
        }
        class = if own { None } else { owner.superclass };
    }
    made.entries = entries.len();
    let list = lua.create_table_with_capacity(entries.len(), 0)?;
    for (at, entry) in entries.into_iter().enumerate() {
        list.raw_set(at + 1, entry)?;
    }
    Ok((frozen(list), made))
}

/// A class's ReflectedClass entry: a new frozen table, as each call makes.
fn class_entry(lua: &Lua, engine: &Engine, data: &Data, index: u32, filter: &ClassFilter) -> LuaResult<LuaTable> {
    let class = &data.classes[index as usize];
    let entry = lua.create_table_with_capacity(0, 6)?;
    entry.raw_set("Name", &*class.name)?;
    if let Some(superclass) = class.superclass {
        entry.raw_set("Superclass", &*data.classes[superclass as usize].name)?;
    }
    if class.serialized {
        entry.raw_set("Serialized", true)?;
    }
    if !class.subclasses.is_empty() {
        let list = lua.create_table_with_capacity(class.subclasses.len(), 0)?;
        for (at, &subclass) in class.subclasses.iter().enumerate() {
            list.raw_set(at + 1, &*data.classes[subclass as usize].name)?;
        }
        entry.raw_set("Subclasses", frozen(list))?;
    }
    if filter.display {
        entry.raw_set("Display", display_table(lua, &class.display)?)?;
    }
    let make: LuaFunction = engine.prelude().exports.raw_get("CapabilitiesOfMask")?;
    let mut count = 0;
    entry.raw_set("Permits", permits_table(lua, &make, &class.permits, filter.security, &mut count)?)?;
    Ok(frozen(entry))
}

/// The work clock's price of what GetClass or GetClasses made and tested (cost.rs REFLECTED_CLASS).
fn charge(engine: &Engine, made: usize, tested: usize, display: bool) {
    let each = crate::cost::REFLECTED_CLASS + if display { crate::cost::REFLECTED_CLASS_DISPLAY } else { 0.0 };
    charge_seconds(engine, made as f64 * each + tested as f64 * crate::cost::REFLECTED_CLASS_TESTED);
}

/// (measured: nil "Argument 2 missing or nil"; any other value is looked up as its text, as a client reads a string parameter:
/// a number's `%.20g`, an Instance "Instance", an EnumItem its name; services::strings)
const CLASS_NAME: &[StringParam] = &[StringParam { missing: None, ..StringParam::at(1) }];

pub fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let reflection = engine.reflection.clone();
    let Some(service) = reflection.class_id("ReflectionService") else { return };
    if let Some(n) = reflection.name_id("GetClasses") {
        behaviors.add_method(service, n, method(|lua, engine, _, args| {
            let table = dictionary(engine, args, 1)?;
            let data = data::data();
            let filter = class_filter(engine, data, table.as_ref())?;
            let found: Vec<u32> = (0..data.classes.len() as u32).filter(|&index| class_matches(data, index, &filter)).collect();
            let list = lua.create_table_with_capacity(found.len(), 0)?;
            for (at, &index) in found.iter().enumerate() {
                list.raw_set(at + 1, class_entry(lua, engine, data, index, &filter)?)?;
            }
            let tested = data.classes.iter().filter(|class| listed(class, filter.security)).count();
            charge(engine, found.len(), tested, filter.display);
            Ok(values([LuaValue::Table(frozen(list))]))
        }));
    }
    if let Some(n) = reflection.name_id("GetClass") {
        behaviors.add_method(service, n, strings(CLASS_NAME, method(|lua, engine, _, args| {
            let Some(name) = args.string(1).map(|name| name.to_string_lossy()) else { return raise("Argument 2 missing or nil") };
            let table = dictionary(engine, args, 2)?;
            let data = data::data();
            // (a name no class has answers nil before the fields are read; a class GetClass never gives, UserGameSettings, has them read)
            let Some(index) = data.index(&name) else { return Ok(values([LuaValue::Nil])) };
            let filter = class_filter(engine, data, table.as_ref())?;
            if !class_matches(data, index, &filter) {
                return Ok(values([LuaValue::Nil]));
            }
            charge(engine, 1, 0, filter.display);
            Ok(values([LuaValue::Table(class_entry(lua, engine, data, index, &filter)?)]))
        })));
    }
    for (name, kind) in [("GetPropertiesOfClass", Kind::Properties), ("GetMethodsOfClass", Kind::Methods), ("GetEventsOfClass", Kind::Events)] {
        let Some(n) = reflection.name_id(name) else { continue };
        behaviors.add_method(service, n, strings(CLASS_NAME, method(move |lua, engine, _, args| {
            let Some(name) = args.string(1).map(|name| name.to_string_lossy()) else { return raise("Argument 2 missing or nil") };
            let table = dictionary(engine, args, 2)?;
            let data = data::data();
            // (a name no class has answers nil before the fields are read; one only a Subclasses list gives has them read)
            let Some(index) = data.index(&name) else { return Ok(values([LuaValue::Nil])) };
            let security = security(engine, table.as_ref(), "ReflectionMemberFilter")?;
            let own = flag(table.as_ref(), "ReflectionMemberFilter", "ExcludeInherited")?;
            let display = !flag(table.as_ref(), "ReflectionMemberFilter", "ExcludeDisplay")?;
            let class = &data.classes[index as usize];
            if !class.answers {
                return Ok(values([LuaValue::Nil]));
            }
            if !data::holds(security, class.members) {
                charge_seconds(engine, crate::cost::REFLECTED_MEMBERS_REFUSED);
                return Ok(values([LuaValue::Nil]));
            }
            let (list, made) = members_list(lua, engine, data, index, kind, security, own, display)?;
            charge_members(engine, kind, &made);
            Ok(values([LuaValue::Table(list)]))
        })));
    }
}
