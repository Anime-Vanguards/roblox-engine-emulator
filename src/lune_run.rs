//! Standalone Lune-compatible Luau runner (`ree lune <script.luau> [args...]` and `ree lune --batch <jobs.json>`).
//!
//! Runs Luau scripts against the seven core `@lune/*` built-in libraries (`@lune/fs`, `@lune/luau`,
//! `@lune/process`, `@lune/roblox`, `@lune/serde`, `@lune/stdio`, `@lune/task`) using `ree`'s
//! patched Luau JIT VM, hybrid fast `longjmp` error recovery, native SIMD `Vector3`, thread-local
//! `lune-roblox` DOM, and `mimalloc`. In `--batch` mode, worker threads share compiled bytecode
//! and immutable `git show <hash>:<path>` results across every job in one process and emit a
//! sound read-set trace (`cacheable`, `files`, `dirs`, `stats`, `env`, `gitShows`) per job.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ffi::{c_int, c_void},
    fs,
    hash::{Hash, Hasher},
    io::Write,
    path::Path,
    process::{Command, Stdio},
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, UNIX_EPOCH},
};

use mlua::{ffi, prelude::*};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;
use serde_json::{Map as JsonMap, Value as Json, json};

use lune_roblox::{
    datatypes::types as rbx_types,
    document::{Document, DocumentFormat, DocumentKind},
    instance::{Instance as RbxInstance, registry::InstanceRegistry, reset_internal_dom},
    reflection::{Database as ReflectionDatabase, DatabaseClass, DatabaseEnum, DatabaseProperty},
};

unsafe extern "C-unwind" {
    fn lua_setworkcounting(L: *mut ffi::lua_State, enabled: c_int);
}

/// Lightweight userdata wrapper for `@lune/roblox`'s reflection database.
///
/// Returning `lune_roblox::reflection::Database` directly instantiates `lune-roblox`'s
/// `DomValueToLua` and `mlua::UserDataRegistry` for every Roblox datatype inside this crate.
/// Delegating to `lune-roblox`'s compiled database methods and `lune_roblox::datatypes::create`
/// preserves the exact Lua API while instantiating only this single userdata type.
#[derive(Clone)]
enum ReflHandle {
    Db(ReflectionDatabase, LuaValue),
    Class(DatabaseClass, LuaValue),
    Enum(DatabaseEnum),
    Prop(DatabaseProperty),
}

fn lune_enum_value(enums: &LuaValue, enum_name: &str, value: u32) -> Option<LuaValue> {
    let db = rbx_reflection_database::get().ok()?;
    let desc = db.enums.get(enum_name)?;
    let (item_name, _) = desc.items.iter().find(|(_, v)| **v == value)?;
    let enum_val: LuaValue = match enums {
        LuaValue::UserData(ud) => ud.get(enum_name).ok()?,
        LuaValue::Table(tbl) => tbl.get(enum_name).ok()?,
        _ => return None,
    };
    match enum_val {
        LuaValue::UserData(ud) => ud.get(item_name.as_ref()).ok(),
        LuaValue::Table(tbl) => tbl.get(item_name.as_ref()).ok(),
        _ => None,
    }
}

fn lune_dom_variant_to_lua(lua: &Lua, enums: &LuaValue, variant: &rbx_dom_weak::types::Variant) -> Option<LuaValue> {
    use lune_roblox::datatypes::create as lune_create;
    use rbx_dom_weak::types::{self as dom, Variant as DomVariant};

    let result: LuaResult<LuaValue> = match variant {
        DomVariant::Vector3(v) => Ok(LuaValue::Vector(mlua::Vector::new(v.x, v.y, v.z))),
        DomVariant::Bool(b) => Ok(LuaValue::Boolean(*b)),
        DomVariant::Int64(i) => Ok(LuaValue::Number(*i as f64)),
        DomVariant::Int32(i) => Ok(LuaValue::Number(*i as f64)),
        DomVariant::Float64(n) => Ok(LuaValue::Number(*n)),
        DomVariant::Float32(n) => Ok(LuaValue::Number(*n as f64)),
        DomVariant::String(s) => lua.create_string(s).map(LuaValue::String),
        DomVariant::BinaryString(s) => lua.create_string(s).map(LuaValue::String),
        DomVariant::ContentId(s) => lua.create_string(AsRef::<str>::as_ref(s)).map(LuaValue::String),
        DomVariant::Ref(value) => match RbxInstance::new_opt(*value) {
            Some(inst) => inst.into_cached_lua(lua),
            None => Ok(LuaValue::Nil),
        },
        DomVariant::OptionalCFrame(None) | DomVariant::PhysicalProperties(dom::PhysicalProperties::Default) => Ok(LuaValue::Nil),
        DomVariant::Axes(v) => lune_create::axes(lua, rbx_types::Axes::from(*v)).map(LuaValue::UserData),
        DomVariant::BrickColor(v) => lune_create::brick_color(lua, rbx_types::BrickColor::from(*v)).map(LuaValue::UserData),
        DomVariant::CFrame(v) | DomVariant::OptionalCFrame(Some(v)) => {
            lune_create::cframe(lua, rbx_types::CFrame::from(*v)).map(LuaValue::UserData)
        }
        DomVariant::Color3(v) => lune_create::color3(lua, rbx_types::Color3::from(*v)).map(LuaValue::UserData),
        DomVariant::Color3uint8(v) => lune_create::color3(lua, rbx_types::Color3::from(*v)).map(LuaValue::UserData),
        DomVariant::ColorSequence(v) => {
            lune_create::color_sequence(lua, rbx_types::ColorSequence::from(v.clone())).map(LuaValue::UserData)
        }
        DomVariant::Content(v) => lune_create::content(lua, rbx_types::Content::from(v.clone())).map(LuaValue::UserData),
        DomVariant::EnumItem(v) => return lune_enum_value(enums, &v.ty, v.value),
        DomVariant::Faces(v) => lune_create::faces(lua, rbx_types::Faces::from(*v)).map(LuaValue::UserData),
        DomVariant::Font(v) => lune_create::font(lua, rbx_types::Font::from(v.clone())).map(LuaValue::UserData),
        DomVariant::NumberRange(v) => lune_create::number_range(lua, rbx_types::NumberRange::from(*v)).map(LuaValue::UserData),
        DomVariant::NumberSequence(v) => {
            lune_create::number_sequence(lua, rbx_types::NumberSequence::from(v.clone())).map(LuaValue::UserData)
        }
        DomVariant::PhysicalProperties(dom::PhysicalProperties::Custom(v)) => {
            lune_create::physical_properties(lua, rbx_types::PhysicalProperties::from(*v)).map(LuaValue::UserData)
        }
        DomVariant::Ray(v) => lune_create::ray(lua, rbx_types::Ray::from(*v)).map(LuaValue::UserData),
        DomVariant::Rect(v) => lune_create::rect(lua, rbx_types::Rect::from(*v)).map(LuaValue::UserData),
        DomVariant::Region3(v) => lune_create::region3(lua, rbx_types::Region3::from(*v)).map(LuaValue::UserData),
        DomVariant::Region3int16(v) => {
            lune_create::region3int16(lua, rbx_types::Region3int16::from(*v)).map(LuaValue::UserData)
        }
        DomVariant::UDim(v) => lune_create::udim(lua, rbx_types::UDim::from(*v)).map(LuaValue::UserData),
        DomVariant::UDim2(v) => lune_create::udim2(lua, rbx_types::UDim2::from(*v)).map(LuaValue::UserData),
        DomVariant::UniqueId(v) => lune_create::unique_id(lua, rbx_types::UniqueId::from(*v)).map(LuaValue::UserData),
        DomVariant::Vector2(v) => lune_create::vector2(lua, rbx_types::Vector2::from(*v)).map(LuaValue::UserData),
        DomVariant::Vector2int16(v) => {
            lune_create::vector2int16(lua, rbx_types::Vector2int16::from(*v)).map(LuaValue::UserData)
        }
        DomVariant::Vector3int16(v) => {
            lune_create::vector3int16(lua, rbx_types::Vector3int16::from(*v)).map(LuaValue::UserData)
        }
        _ => return None,
    };
    result.ok()
}

fn lune_default_properties(lua: &Lua, class: DatabaseClass, enums: &LuaValue) -> LuaResult<LuaTable> {
    let class_name = class.get_name();
    let rbx_db = rbx_reflection_database::get().ok();
    let class_desc = rbx_db.and_then(|d| d.classes.get(class_name.as_str()));
    let defaults = class.get_defaults();
    let out = lua.create_table_with_capacity(0, defaults.len())?;
    for (name, prop) in defaults {
        let value = if let rbx_dom_weak::types::Variant::Enum(enum_value) = prop {
            class_desc.and_then(|cd| {
                let prop_desc = cd.properties.get(name.as_str())?;
                if let rbx_reflection::DataType::Enum(enum_name) = &prop_desc.data_type {
                    lune_enum_value(enums, enum_name.as_ref(), enum_value.to_u32())
                } else {
                    None
                }
            })
        } else {
            lune_dom_variant_to_lua(lua, enums, &prop)
        };
        if let Some(value) = value {
            out.raw_set(name, value)?;
        }
    }
    Ok(out)
}

impl LuaUserData for ReflHandle {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Eq, |_, this, other: LuaAnyUserData| {
            let Ok(other) = other.borrow::<ReflHandle>() else {
                return Ok(false);
            };
            Ok(match (this, &*other) {
                (ReflHandle::Db(_, _), ReflHandle::Db(_, _)) => true,
                (ReflHandle::Class(a, _), ReflHandle::Class(b, _)) => a == b,
                (ReflHandle::Enum(a), ReflHandle::Enum(b)) => a == b,
                (ReflHandle::Prop(a), ReflHandle::Prop(b)) => a == b,
                _ => false,
            })
        });
        methods.add_meta_method(LuaMetaMethod::ToString, |_, this, ()| {
            Ok(match this {
                ReflHandle::Db(db, _) => db.to_string(),
                ReflHandle::Class(c, _) => c.to_string(),
                ReflHandle::Enum(e) => e.to_string(),
                ReflHandle::Prop(p) => p.to_string(),
            })
        });
        methods.add_meta_method(LuaMetaMethod::Index, |lua, this, key: String| match this {
            ReflHandle::Db(db, enums) => {
                let db = *db;
                match key.as_str() {
                    "Version" => db.get_version().into_lua(lua),
                    "GetEnumNames" => crate::lua::lua_fn(lua, move |lua, _self: LuaValue| db.get_enum_names().into_lua(lua))
                        .map(LuaValue::Function),
                    "GetClassNames" => crate::lua::lua_fn(lua, move |lua, _self: LuaValue| db.get_class_names().into_lua(lua))
                        .map(LuaValue::Function),
                    "GetEnum" => crate::lua::lua_fn(lua, move |lua, (_self, name): (LuaValue, String)| match db.get_enum(name) {
                        Some(e) => lua.create_userdata(ReflHandle::Enum(e)).map(LuaValue::UserData),
                        None => Ok(LuaValue::Nil),
                    })
                    .map(LuaValue::Function),
                    "GetClass" => {
                        let enums = enums.clone();
                        crate::lua::lua_fn(lua, move |lua, (_self, name): (LuaValue, String)| match db.get_class(name) {
                            Some(c) => lua.create_userdata(ReflHandle::Class(c, enums.clone())).map(LuaValue::UserData),
                            None => Ok(LuaValue::Nil),
                        })
                        .map(LuaValue::Function)
                    }
                    "FindEnum" => crate::lua::lua_fn(lua, move |lua, (_self, name): (LuaValue, String)| match db.find_enum(name) {
                        Some(e) => lua.create_userdata(ReflHandle::Enum(e)).map(LuaValue::UserData),
                        None => Ok(LuaValue::Nil),
                    })
                    .map(LuaValue::Function),
                    "FindClass" => {
                        let enums = enums.clone();
                        crate::lua::lua_fn(lua, move |lua, (_self, name): (LuaValue, String)| match db.find_class(name) {
                            Some(c) => lua.create_userdata(ReflHandle::Class(c, enums.clone())).map(LuaValue::UserData),
                            None => Ok(LuaValue::Nil),
                        })
                        .map(LuaValue::Function)
                    }
                    _ => Ok(LuaValue::Nil),
                }
            }
            ReflHandle::Enum(e) => match key.as_str() {
                "Name" => e.get_name().into_lua(lua),
                "Items" => {
                    let items = e.get_items();
                    let tbl = lua.create_table_with_capacity(0, items.len())?;
                    for (name, val) in items {
                        tbl.raw_set(name, val)?;
                    }
                    Ok(LuaValue::Table(tbl))
                }
                _ => Ok(LuaValue::Nil),
            },
            ReflHandle::Class(c, enums) => match key.as_str() {
                "Name" => c.get_name().into_lua(lua),
                "Superclass" => c.get_superclass().into_lua(lua),
                "Tags" => c.get_tags_str().into_lua(lua),
                "Properties" => {
                    let props = c.get_properties();
                    let tbl = lua.create_table_with_capacity(0, props.len())?;
                    for (name, prop) in props {
                        tbl.raw_set(name, lua.create_userdata(ReflHandle::Prop(prop))?)?;
                    }
                    Ok(LuaValue::Table(tbl))
                }
                "DefaultProperties" => lune_default_properties(lua, *c, enums).map(LuaValue::Table),
                _ => Ok(LuaValue::Nil),
            },
            ReflHandle::Prop(p) => match key.as_str() {
                "Name" => p.get_name().into_lua(lua),
                "Datatype" => p.get_datatype_name().into_lua(lua),
                "Scriptability" => p.get_scriptability_str().into_lua(lua),
                "Tags" => p.get_tags_str().into_lua(lua),
                _ => Ok(LuaValue::Nil),
            },
        });
    }
}

// ---------------------------------------------------------------------------
// Process-wide shared caches across `--batch` workers
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct CachedBytecode {
    source: Arc<[u8]>,
    opt: u8,
    dbg: u8,
    cov: u8,
    bytecode: Arc<[u8]>,
}

static BYTECODE_CACHE: OnceLock<RwLock<FxHashMap<u64, Vec<CachedBytecode>>>> = OnceLock::new();

#[derive(Clone)]
struct CachedProcessOutput {
    ok: bool,
    code: i32,
    stdout: Arc<[u8]>,
    stderr: Arc<[u8]>,
}

static GIT_SHOW_CACHE: OnceLock<RwLock<FxHashMap<String, CachedProcessOutput>>> = OnceLock::new();

// 64-way sharded read-only filesystem cache scoped to a single `ree lune --batch` run.
// Avoids repeating hundreds of thousands of Windows NTFS opens across repo-walking census tests.
// Automatically disabled if any job writes/removes/moves inside the repository.
const FS_CACHE_SHARDS: usize = 64;
const FS_CACHE_MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy)]
enum CachedStat {
    File,
    Dir,
    Missing,
}

impl CachedStat {
    fn as_str(self) -> &'static str {
        match self {
            CachedStat::File => "file",
            CachedStat::Dir => "dir",
            CachedStat::Missing => "missing",
        }
    }
}

struct BatchFsCache {
    disabled: AtomicBool,
    cached_bytes: AtomicUsize,
    files: [RwLock<FxHashMap<String, Option<Arc<[u8]>>>>; FS_CACHE_SHARDS],
    dirs: [RwLock<FxHashMap<String, Option<Arc<[String]>>>>; FS_CACHE_SHARDS],
    stats: [RwLock<FxHashMap<String, CachedStat>>; FS_CACHE_SHARDS],
}

impl BatchFsCache {
    fn new() -> Self {
        Self {
            disabled: AtomicBool::new(false),
            cached_bytes: AtomicUsize::new(0),
            files: std::array::from_fn(|_| RwLock::new(FxHashMap::default())),
            dirs: std::array::from_fn(|_| RwLock::new(FxHashMap::default())),
            stats: std::array::from_fn(|_| RwLock::new(FxHashMap::default())),
        }
    }

    fn shard_of(key: &str) -> usize {
        let mut h = rustc_hash::FxHasher::default();
        key.hash(&mut h);
        (h.finish() as usize) & (FS_CACHE_SHARDS - 1)
    }

    fn invalidate_if_repo_path(&self, raw_path: &str) {
        let norm = normalize_path(raw_path);
        if is_inside_repo(&norm) {
            self.disabled.store(true, Ordering::Release);
        }
    }

    fn stat(&self, norm: &str, raw_path: &str) -> CachedStat {
        if self.disabled.load(Ordering::Acquire) || !is_inside_repo(norm) {
            return os_stat(raw_path);
        }
        let shard = Self::shard_of(norm);
        if let Ok(guard) = self.stats[shard].read()
            && let Some(&hit) = guard.get(norm)
        {
            return hit;
        }
        let val = os_stat(raw_path);
        if !self.disabled.load(Ordering::Acquire)
            && let Ok(mut guard) = self.stats[shard].write()
        {
            guard.insert(norm.to_string(), val);
        }
        val
    }

    fn read_file(&self, norm: &str, raw_path: &str) -> std::io::Result<Arc<[u8]>> {
        if self.disabled.load(Ordering::Acquire) || !is_inside_repo(norm) {
            return fs::read(raw_path).map(|v| Arc::from(v.as_slice()));
        }
        let shard = Self::shard_of(norm);
        if let Ok(guard) = self.files[shard].read()
            && let Some(hit) = guard.get(norm)
        {
            return match hit {
                Some(bytes) => Ok(bytes.clone()),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("cached missing file: {raw_path}"),
                )),
            };
        }
        match fs::read(raw_path) {
            Ok(bytes) => {
                let arc: Arc<[u8]> = Arc::from(bytes.as_slice());
                let prev = self.cached_bytes.fetch_add(arc.len(), Ordering::Relaxed);
                if prev + arc.len() <= FS_CACHE_MAX_BYTES
                    && !self.disabled.load(Ordering::Acquire)
                    && let Ok(mut guard) = self.files[shard].write()
                {
                    guard.insert(norm.to_string(), Some(arc.clone()));
                } else {
                    self.cached_bytes.fetch_sub(arc.len(), Ordering::Relaxed);
                }
                Ok(arc)
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound
                    && !self.disabled.load(Ordering::Acquire)
                    && let Ok(mut guard) = self.files[shard].write()
                {
                    guard.insert(norm.to_string(), None);
                }
                Err(e)
            }
        }
    }

    fn read_dir(&self, norm: &str, raw_path: &str) -> std::io::Result<Arc<[String]>> {
        if self.disabled.load(Ordering::Acquire) || !is_inside_repo(norm) {
            return os_read_dir(raw_path).map(|v| Arc::from(v.into_boxed_slice()));
        }
        let shard = Self::shard_of(norm);
        if let Ok(guard) = self.dirs[shard].read()
            && let Some(hit) = guard.get(norm)
        {
            return match hit {
                Some(entries) => Ok(entries.clone()),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("cached missing dir: {raw_path}"),
                )),
            };
        }
        match os_read_dir(raw_path) {
            Ok(entries) => {
                let arc: Arc<[String]> = Arc::from(entries.into_boxed_slice());
                if !self.disabled.load(Ordering::Acquire)
                    && let Ok(mut guard) = self.dirs[shard].write()
                {
                    guard.insert(norm.to_string(), Some(arc.clone()));
                }
                Ok(arc)
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound
                    && !self.disabled.load(Ordering::Acquire)
                    && let Ok(mut guard) = self.dirs[shard].write()
                {
                    guard.insert(norm.to_string(), None);
                }
                Err(e)
            }
        }
    }
}

fn os_stat(raw_path: &str) -> CachedStat {
    match fs::metadata(raw_path) {
        Ok(m) if m.is_file() => CachedStat::File,
        Ok(m) if m.is_dir() => CachedStat::Dir,
        _ => CachedStat::Missing,
    }
}

fn os_read_dir(raw_path: &str) -> std::io::Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(raw_path)? {
        let entry = entry?;
        entries.push(entry.file_name().to_string_lossy().into_owned());
    }
    Ok(entries)
}

fn hash_bytes(bytes: &[u8], opt: u8, dbg: u8, cov: u8) -> u64 {
    let mut hasher = rustc_hash::FxHasher::default();
    bytes.hash(&mut hasher);
    opt.hash(&mut hasher);
    dbg.hash(&mut hasher);
    cov.hash(&mut hasher);
    hasher.finish()
}

fn compile_cached(source: &[u8], opt: u8, dbg: u8, cov: u8) -> LuaResult<Arc<[u8]>> {
    let cache = BYTECODE_CACHE.get_or_init(|| RwLock::new(FxHashMap::default()));
    let key = hash_bytes(source, opt, dbg, cov);
    if let Ok(guard) = cache.read()
        && let Some(bucket) = guard.get(&key)
        && let Some(entry) = bucket.iter().find(|e| e.opt == opt && e.dbg == dbg && e.cov == cov && e.source.as_ref() == source)
    {
        return Ok(entry.bytecode.clone());
    }
    let compiled = mlua::chunk::Compiler::new()
        .set_optimization_level(opt)
        .set_debug_level(dbg)
        .set_coverage_level(cov)
        .compile(source)?;
    let bytecode: Arc<[u8]> = Arc::from(compiled.as_slice());
    let entry = CachedBytecode {
        source: Arc::from(source),
        opt,
        dbg,
        cov,
        bytecode: bytecode.clone(),
    };
    if let Ok(mut guard) = cache.write() {
        let bucket = guard.entry(key).or_default();
        if !bucket.iter().any(|e| e.opt == opt && e.dbg == dbg && e.cov == cov && e.source.as_ref() == source) {
            bucket.push(entry);
        }
    }
    Ok(bytecode)
}

fn is_luau_bytecode(bytes: &[u8]) -> bool {
    matches!(bytes, [3, ..] | [4..=20, 1..=3, ..])
}

fn apply_codegen(lua: &Lua, func: &LuaFunction, enabled: bool) {
    if !enabled {
        return;
    }
    #[cfg(feature = "jit")]
    unsafe {
        if ffi::luau_codegen_supported() != 0 {
            let _ = lua.exec_raw::<()>(func, |state| {
                ffi::luau_codegen_compile(state, -1);
            });
        }
    }
    #[cfg(not(feature = "jit"))]
    let _ = (lua, func);
}

// ---------------------------------------------------------------------------
// Path normalization & sound read-set tracer (matches tools/lune-trace.luau)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TraceState {
    enabled: bool,
    cacheable: bool,
    files: FxHashSet<String>,
    dirs: FxHashSet<String>,
    stats: FxHashMap<String, &'static str>,
    env: FxHashMap<String, Option<String>>,
    git_shows: FxHashSet<String>,
}

impl TraceState {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            cacheable: true,
            ..Self::default()
        }
    }

    fn record_stat_known(&mut self, norm: String, kind: CachedStat) {
        if !self.enabled {
            return;
        }
        if !is_inside_repo(&norm) {
            self.cacheable = false;
            return;
        }
        self.stats.entry(norm).or_insert_with(|| kind.as_str());
    }

    fn record_stat(&mut self, raw_path: &str) {
        if !self.enabled {
            return;
        }
        let norm = normalize_path(raw_path);
        if !is_inside_repo(&norm) {
            self.cacheable = false;
            return;
        }
        self.stats.entry(norm).or_insert_with(|| os_stat(raw_path).as_str());
    }

    fn record_file_read_norm(&mut self, norm: &str) {
        if !self.enabled {
            return;
        }
        if !is_inside_repo(norm) {
            self.cacheable = false;
            return;
        }
        if !self.files.contains(norm) {
            self.files.insert(norm.to_string());
        }
    }

    fn record_dir_read_norm(&mut self, norm: &str) {
        if !self.enabled {
            return;
        }
        if !is_inside_repo(norm) {
            self.cacheable = false;
            return;
        }
        if !self.dirs.contains(norm) {
            self.dirs.insert(norm.to_string());
        }
    }

    fn record_env(&mut self, key: &str, value: Option<String>) {
        if !self.enabled {
            return;
        }
        self.env.entry(key.to_string()).or_insert(value);
    }

    fn taint(&mut self) {
        if self.enabled {
            self.cacheable = false;
        }
    }

    fn to_json(&self, passed: bool) -> Json {
        let mut env_rows: Vec<(&String, &Option<String>)> = self.env.iter().collect();
        crate::sort::by(&mut env_rows, |a, b| a.0.cmp(b.0));
        let env_pairs: Vec<Json> = env_rows
            .into_iter()
            .map(|(k, v)| match v {
                Some(s) => json!([k, s]),
                None => json!([k, false]),
            })
            .collect();
        let mut files: Vec<&String> = self.files.iter().collect();
        crate::sort::ord(&mut files);
        let mut dirs: Vec<&String> = self.dirs.iter().collect();
        crate::sort::ord(&mut dirs);
        let mut git_shows: Vec<&String> = self.git_shows.iter().collect();
        crate::sort::ord(&mut git_shows);
        let mut stats_rows: Vec<(&String, &&'static str)> = self.stats.iter().collect();
        crate::sort::by(&mut stats_rows, |a, b| a.0.cmp(b.0));
        let mut stats = JsonMap::with_capacity(stats_rows.len());
        for (k, v) in stats_rows {
            stats.insert(k.clone(), Json::String((*v).to_string()));
        }
        json!({
            "cacheable": passed && self.cacheable,
            "files": files,
            "dirs": dirs,
            "stats": Json::Object(stats),
            "env": env_pairs,
            "gitShows": git_shows,
        })
    }
}

fn normalize_path(raw: &str) -> String {
    let mut path = raw.replace('\\', "/");
    let prefix = if path.starts_with('/') {
        "/".to_string()
    } else if path.len() >= 3
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path.as_bytes()[1] == b':'
        && path.as_bytes()[2] == b'/'
    {
        let p = path[..3].to_string();
        path = path[3..].to_string();
        p
    } else {
        String::new()
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        } else if part == ".." {
            if let Some(&last) = parts.last()
                && last != ".."
            {
                parts.pop();
            } else if prefix.is_empty() {
                parts.push("..");
            }
        } else {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        return if prefix.is_empty() { ".".to_string() } else { prefix };
    }
    format!("{prefix}{}", parts.join("/"))
}

fn dir_name(normalized: &str) -> String {
    match normalized.rfind('/') {
        Some(0) => "/".to_string(),
        Some(pos) => normalized[..pos].to_string(),
        None => ".".to_string(),
    }
}

fn join_norm(base_dir: &str, rel: &str) -> String {
    if base_dir.is_empty() || base_dir == "." {
        normalize_path(rel)
    } else {
        normalize_path(&format!("{base_dir}/{rel}"))
    }
}

fn is_inside_repo(normalized: &str) -> bool {
    if normalized.starts_with('/') {
        return false;
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/' {
        return false;
    }
    if normalized == ".." || normalized.starts_with("../") {
        return false;
    }
    true
}

fn parse_immutable_git_show(program: &str, args: &[String]) -> Option<String> {
    if program != "git" {
        return None;
    }
    let start = if args.len() >= 2 && args[0] == "-C" && args[1] == "." { 2 } else { 0 };
    if args.len() != start + 2 || args[start] != "show" {
        return None;
    }
    let spec = &args[start + 1];
    let (commit, file_path) = spec.split_once(':')?;
    if !(7..=40).contains(&commit.len())
        || !commit.bytes().all(|b| b.is_ascii_hexdigit())
        || file_path.is_empty()
    {
        return None;
    }
    Some(format!("{commit}:{file_path}"))
}

// ---------------------------------------------------------------------------
// Output sink & per-VM runtime state
// ---------------------------------------------------------------------------

struct OutputSink {
    capture: bool,
    buf: RefCell<String>,
}

fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

impl OutputSink {
    fn new(capture: bool) -> Self {
        Self {
            capture,
            buf: RefCell::new(String::new()),
        }
    }

    fn write_stdout(&self, text: &str) {
        if self.capture {
            self.append(text);
        } else {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(text.as_bytes());
        }
    }

    fn write_stderr(&self, text: &str) {
        if self.capture {
            self.append(text);
        } else {
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(text.as_bytes());
        }
    }

    fn append(&self, text: &str) {
        const MAX_CAPTURE: usize = 512 * 1024;
        let mut buf = self.buf.borrow_mut();
        buf.push_str(text);
        if buf.len() > MAX_CAPTURE {
            let cut = buf.len() - MAX_CAPTURE / 2;
            let boundary = ceil_char_boundary(&buf, cut);
            buf.drain(..boundary);
        }
    }

    fn take(&self) -> String {
        std::mem::take(&mut *self.buf.borrow_mut())
    }
}

struct TimerEntry {
    deadline: Instant,
    started: Instant,
    thread: LuaThread,
    args: Vec<LuaValue>,
    is_wait: bool,
}

struct TaskQueue {
    deferred: VecDeque<(LuaThread, Vec<LuaValue>)>,
    timers: Vec<TimerEntry>,
    cancelled: FxHashSet<*const c_void>,
}

impl TaskQueue {
    fn new() -> Self {
        Self {
            deferred: VecDeque::new(),
            timers: Vec::new(),
            cancelled: FxHashSet::default(),
        }
    }
}

struct LuneRuntime {
    trace: RefCell<TraceState>,
    output: OutputSink,
    exit_code: Cell<Option<i32>>,
    tasks: RefCell<TaskQueue>,
    module_cache: RefCell<FxHashMap<String, LuaRegistryKey>>,
    loading: RefCell<FxHashSet<String>>,
    builtins: RefCell<Option<LuaTable>>,
    env_helpers: RefCell<Option<Rc<EnvHelpers>>>,
    fs_cache: Option<Arc<BatchFsCache>>,
    jit_loads_remaining: Cell<usize>,
    bytecode_loads: RefCell<FxHashMap<u64, u32>>,
    args: Vec<String>,
    cwd: String,
}

impl LuneRuntime {
    fn cached_stat(&self, raw_path: &str) -> (String, CachedStat) {
        let norm = normalize_path(raw_path);
        let kind = match &self.fs_cache {
            Some(c) => c.stat(&norm, raw_path),
            None => os_stat(raw_path),
        };
        (norm, kind)
    }

    fn cached_read_file(&self, raw_path: &str) -> std::io::Result<(String, Arc<[u8]>)> {
        let norm = normalize_path(raw_path);
        let bytes = match &self.fs_cache {
            Some(c) => c.read_file(&norm, raw_path)?,
            None => Arc::from(fs::read(raw_path)?.as_slice()),
        };
        Ok((norm, bytes))
    }

    fn cached_read_dir(&self, raw_path: &str) -> std::io::Result<(String, Arc<[String]>)> {
        let norm = normalize_path(raw_path);
        let entries = match &self.fs_cache {
            Some(c) => c.read_dir(&norm, raw_path)?,
            None => Arc::from(os_read_dir(raw_path)?.into_boxed_slice()),
        };
        Ok((norm, entries))
    }
}

#[derive(Debug)]
struct ExitSignal(i32);

impl std::fmt::Display for ExitSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "process.exit({})", self.0)
    }
}

impl std::error::Error for ExitSignal {}

// ---------------------------------------------------------------------------
// Native SIMD Vector3 & @lune/roblox setup
// ---------------------------------------------------------------------------

fn install_typeof(lua: &Lua) -> LuaResult<()> {
    let registry = lua.create_table()?;
    let typeof_fn = crate::lua::raw::typeof_function(lua, registry)?;
    lua.globals().raw_set("typeof", typeof_fn)?;
    Ok(())
}

fn install_native_vector3(lua: &Lua, roblox: &LuaTable) -> LuaResult<()> {
    let methods = crate::lua::vector_methods::methods(lua)?;

    let fuzzy_eq = crate::lua::lua_fn(lua, |lua, args: LuaMultiValue| {
        let Some(LuaValue::Vector(a)) = args.front() else {
            return Err(LuaError::runtime("invalid argument #1 to 'FuzzyEq' (Vector3 expected)"));
        };
        let Some(LuaValue::Vector(b)) = args.get(1) else {
            return Err(LuaError::runtime("invalid argument #2 to 'FuzzyEq' (Vector3 expected)"));
        };
        let eps = match args.get(2) {
            None | Some(LuaValue::Nil) => 1e-5f32,
            Some(v) => lua.coerce_number(v.clone())?.unwrap_or(1e-5) as f32,
        };
        Ok((a.x() - b.x()).abs() <= eps
            && (a.y() - b.y()).abs() <= eps
            && (a.z() - b.z()).abs() <= eps)
    })?;

    let angle = crate::lua::lua_fn(lua, |_, args: LuaMultiValue| {
        if args.len() < 2 {
            return Err(LuaError::runtime(format!(
                "Invalid number of arguments: {}, Vector3.Angle expect 2 or 3",
                args.len()
            )));
        }
        let Some(LuaValue::Vector(a)) = args.front() else {
            return Err(LuaError::runtime("invalid argument #1 to 'Angle' (Vector3 expected)"));
        };
        let Some(LuaValue::Vector(b)) = args.get(1) else {
            return Err(LuaError::runtime("invalid argument #2 to 'Angle' (Vector3 expected)"));
        };
        let (ax, ay, az) = (a.x(), a.y(), a.z());
        let (bx, by, bz) = (b.x(), b.y(), b.z());
        let cx = ay * bz - az * by;
        let cy = az * bx - ax * bz;
        let cz = ax * by - ay * bx;
        let cross_mag = (cx * cx + cy * cy + cz * cz).sqrt();
        let dot = ax * bx + ay * by + az * bz;
        let mut rad = cross_mag.atan2(dot);
        if let Some(LuaValue::Vector(axis)) = args.get(2)
            && cx * axis.x() + cy * axis.y() + cz * axis.z() < 0.0
        {
            rad = -rad;
        }
        Ok(f64::from(rad))
    })?;

    methods.raw_set("FuzzyEq", fuzzy_eq.clone())?;
    methods.raw_set("Angle", angle.clone())?;

    let extras = lua.create_table()?;
    extras.raw_set("FuzzyEq", fuzzy_eq)?;
    extras.raw_set("Angle", angle)?;

    let methods_mt = lua.create_table()?;
    methods_mt.raw_set(
        "__index",
        crate::lua::lua_fn(lua, |_, (_, key): (LuaValue, LuaValue)| -> LuaResult<LuaValue> {
            match key {
                LuaValue::String(s) => Err(LuaError::runtime(format!(
                    "{} is not a valid member of Vector3",
                    s.to_string_lossy()
                ))),
                other => Err(LuaError::runtime(format!(
                    "invalid argument #2 (string expected, got {})",
                    other.type_name()
                ))),
            }
        })?,
    )?;
    methods.set_metatable(Some(methods_mt))?;

    let vec_mt = lua.create_table()?;
    vec_mt.raw_set("__index", methods)?;
    vec_mt.raw_set("__namecall", crate::lua::vector_methods::namecall(lua, extras)?)?;
    vec_mt.raw_set(
        "__newindex",
        crate::lua::lua_fn(lua, |_, (_, key): (LuaValue, LuaValue)| -> LuaResult<()> {
            Err(LuaError::runtime(format!("{} cannot be assigned to", key.to_string()?)))
        })?,
    )?;
    vec_mt.raw_set("__metatable", "The metatable is locked")?;
    vec_mt.raw_set("__type", "Vector3")?;
    lua.set_type_metatable::<mlua::Vector>(Some(vec_mt));

    let vector_lib: LuaTable = lua.globals().get("vector")?;
    let v3 = lua.create_table()?;
    v3.raw_set(
        "new",
        crate::lua::lua_fn(lua, |_, (x, y, z): (Option<f32>, Option<f32>, Option<f32>)| {
            Ok(LuaValue::Vector(mlua::Vector::new(
                x.unwrap_or(0.0),
                y.unwrap_or(0.0),
                z.unwrap_or(0.0),
            )))
        })?,
    )?;
    v3.raw_set("zero", LuaValue::Vector(mlua::Vector::new(0.0, 0.0, 0.0)))?;
    v3.raw_set("one", LuaValue::Vector(mlua::Vector::new(1.0, 1.0, 1.0)))?;
    v3.raw_set("xAxis", LuaValue::Vector(mlua::Vector::new(1.0, 0.0, 0.0)))?;
    v3.raw_set("yAxis", LuaValue::Vector(mlua::Vector::new(0.0, 1.0, 0.0)))?;
    v3.raw_set("zAxis", LuaValue::Vector(mlua::Vector::new(0.0, 0.0, 1.0)))?;
    v3.raw_set("min", vector_lib.get::<LuaValue>("min")?)?;
    v3.raw_set("max", vector_lib.get::<LuaValue>("max")?)?;

    let from_normal_id = crate::lua::lua_fn(lua, |_, arg: LuaValue| {
        let name: String = match &arg {
            LuaValue::UserData(ud) => {
                let dom: rbx_dom_weak::types::EnumItem = ud.borrow::<rbx_types::EnumItem>()?.clone().into();
                if dom.ty != "NormalId" {
                    return Err(LuaError::runtime(format!("Unable to cast {} to NormalId", dom.ty)));
                }
                ud.get::<String>("Name")?
            }
            LuaValue::Number(n) => match n.trunc() as i64 {
                0 => "Right".into(),
                1 => "Top".into(),
                2 => "Back".into(),
                3 => "Left".into(),
                4 => "Bottom".into(),
                5 => "Front".into(),
                _ => return Err(LuaError::runtime("Invalid NormalId")),
            },
            LuaValue::Integer(n) => match *n {
                0 => "Right".into(),
                1 => "Top".into(),
                2 => "Back".into(),
                3 => "Left".into(),
                4 => "Bottom".into(),
                5 => "Front".into(),
                _ => return Err(LuaError::runtime("Invalid NormalId")),
            },
            LuaValue::String(s) => s.to_str()?.to_string(),
            _ => return Err(LuaError::runtime("invalid argument #1 to 'FromNormalId' (NormalId expected)")),
        };
        let v = match name.as_str() {
            "Right" => mlua::Vector::new(1.0, 0.0, 0.0),
            "Left" => mlua::Vector::new(-1.0, 0.0, 0.0),
            "Top" => mlua::Vector::new(0.0, 1.0, 0.0),
            "Bottom" => mlua::Vector::new(0.0, -1.0, 0.0),
            "Back" => mlua::Vector::new(0.0, 0.0, 1.0),
            "Front" => mlua::Vector::new(0.0, 0.0, -1.0),
            _ => return Err(LuaError::runtime(format!("Invalid NormalId '{name}'"))),
        };
        Ok(LuaValue::Vector(v))
    })?;
    v3.raw_set("FromNormalId", from_normal_id.clone())?;
    v3.raw_set("fromNormalId", from_normal_id)?;

    let from_axis = crate::lua::lua_fn(lua, |_, arg: LuaValue| {
        let name: String = match &arg {
            LuaValue::UserData(ud) => {
                let dom: rbx_dom_weak::types::EnumItem = ud.borrow::<rbx_types::EnumItem>()?.clone().into();
                if dom.ty != "Axis" {
                    return Err(LuaError::runtime(format!("Unable to cast {} to Axis", dom.ty)));
                }
                ud.get::<String>("Name")?
            }
            LuaValue::Number(n) => match n.trunc() as i64 {
                0 => "X".into(),
                1 => "Y".into(),
                2 => "Z".into(),
                _ => return Err(LuaError::runtime("Invalid Axis")),
            },
            LuaValue::Integer(n) => match *n {
                0 => "X".into(),
                1 => "Y".into(),
                2 => "Z".into(),
                _ => return Err(LuaError::runtime("Invalid Axis")),
            },
            LuaValue::String(s) => s.to_str()?.to_string(),
            _ => return Err(LuaError::runtime("invalid argument #1 to 'FromAxis' (Axis expected)")),
        };
        let v = match name.as_str() {
            "X" => mlua::Vector::new(1.0, 0.0, 0.0),
            "Y" => mlua::Vector::new(0.0, 1.0, 0.0),
            "Z" => mlua::Vector::new(0.0, 0.0, 1.0),
            _ => return Err(LuaError::runtime(format!("Invalid Axis '{name}'"))),
        };
        Ok(LuaValue::Vector(v))
    })?;
    v3.raw_set("FromAxis", from_axis.clone())?;
    v3.raw_set("fromAxis", from_axis)?;
    v3.set_readonly(true);

    roblox.raw_set("Vector3", v3)?;
    Ok(())
}

/// The standalone place or model behind `bytes`: a Roblox archive (.rbx) flattened first, since the parser would read
/// its stored scene without the values it keeps outside it (crate::project::decode_model). What the archive's reader
/// noted goes to stderr, as `ree lune`'s other warnings do; an archive it refuses is the call's error.
fn unwrap_archive(bytes: Vec<u8>, function: &str) -> LuaResult<Vec<u8>> {
    let flattened = match ror_archive::unwrap(&bytes) {
        Ok(scene) if scene.package.is_none() => None,
        Ok(scene) => {
            for note in crate::project::archive_notes(&scene.report) {
                eprintln!("roblox.{function}: {note}");
            }
            Some(scene.bytes.into_owned())
        }
        // (not a Roblox file or an archive: the parser's error says so)
        Err(ror_archive::Error::NotArchive) => None,
        Err(error) => return Err(LuaError::runtime(format!("roblox.{function}: Roblox archive: {error}"))),
    };
    Ok(flattened.unwrap_or(bytes))
}

fn bytes_from_lua(val: &LuaValue) -> LuaResult<Vec<u8>> {
    match val {
        LuaValue::String(s) => Ok(s.as_bytes().to_vec()),
        LuaValue::Buffer(b) => Ok(b.to_vec()),
        other => Err(LuaError::runtime(format!(
            "expected string or buffer, got {}",
            other.type_name()
        ))),
    }
}

fn build_roblox_module(lua: &Lua) -> LuaResult<LuaTable> {
    let base = lune_roblox::module(lua.clone())?;
    let module = lua.create_table()?;
    for pair in base.pairs::<LuaValue, LuaValue>() {
        let (k, v) = pair?;
        module.raw_set(k, v)?;
    }

    install_native_vector3(lua, &module)?;

    module.raw_set(
        "deserializeModel",
        crate::lua::lua_fn(lua, |lua, data: LuaValue| {
            let bytes = unwrap_archive(bytes_from_lua(&data)?, "deserializeModel")?;
            let doc = Document::from_bytes(bytes, DocumentKind::Model).map_err(LuaError::external)?;
            let instances = doc.into_instance_array().map_err(LuaError::external)?;
            RbxInstance::vec_into_cached_lua(instances, lua)
        })?,
    )?;
    module.raw_set(
        "deserializePlace",
        crate::lua::lua_fn(lua, |lua, data: LuaValue| {
            let bytes = unwrap_archive(bytes_from_lua(&data)?, "deserializePlace")?;
            let doc = Document::from_bytes(bytes, DocumentKind::Place).map_err(LuaError::external)?;
            let dm = doc.into_data_model_instance().map_err(LuaError::external)?;
            dm.into_cached_lua(lua)
        })?,
    )?;
    module.raw_set(
        "serializeModel",
        crate::lua::lua_fn(lua, |lua, (instances_tbl, xml): (LuaTable, Option<bool>)| {
            let mut instances = Vec::new();
            for item in instances_tbl.sequence_values::<LuaAnyUserData>() {
                let ud = item?;
                instances.push(*ud.borrow::<RbxInstance>()?);
            }
            let doc = Document::from_instance_array(instances).map_err(LuaError::external)?;
            let fmt = if xml.unwrap_or(false) { DocumentFormat::Xml } else { DocumentFormat::Binary };
            let bytes = doc.to_bytes_with_format(fmt).map_err(LuaError::external)?;
            lua.create_string(&bytes)
        })?,
    )?;
    module.raw_set(
        "serializePlace",
        crate::lua::lua_fn(lua, |lua, (dm_ud, xml): (LuaAnyUserData, Option<bool>)| {
            let dm = *dm_ud.borrow::<RbxInstance>()?;
            let doc = Document::from_data_model_instance(dm).map_err(LuaError::external)?;
            let fmt = if xml.unwrap_or(false) { DocumentFormat::Xml } else { DocumentFormat::Binary };
            let bytes = doc.to_bytes_with_format(fmt).map_err(LuaError::external)?;
            lua.create_string(&bytes)
        })?,
    )?;
    let raw_enums: LuaValue = base.raw_get("Enum")?;
    module.raw_set(
        "getReflectionDatabase",
        crate::lua::lua_fn(lua, move |lua, ()| {
            lua.create_userdata(ReflHandle::Db(ReflectionDatabase::new(), raw_enums.clone())).map(LuaValue::UserData)
        })?,
    )?;
    module.raw_set(
        "implementMethod",
        crate::lua::lua_fn(lua, |lua, (class_name, method_name, callback): (String, String, LuaFunction)| {
            InstanceRegistry::insert_method(lua, &class_name, &method_name, callback)
                .map_err(LuaError::external)
        })?,
    )?;
    module.raw_set(
        "implementProperty",
        crate::lua::lua_fn(lua, 
            |lua, (class_name, prop_name, getter, setter): (String, String, LuaFunction, Option<LuaFunction>)| {
                InstanceRegistry::insert_property_getter(lua, &class_name, &prop_name, getter)
                    .map_err(LuaError::external)?;
                if let Some(setter) = setter {
                    InstanceRegistry::insert_property_setter(lua, &class_name, &prop_name, setter)
                        .map_err(LuaError::external)?;
                }
                Ok(())
            },
        )?,
    )?;

    // Lune 0.10.5 NumberRange.new(min, max) swaps reversed bounds rather than raising.
    let nr_tbl = lua.create_table_with_capacity(0, 1)?;
    let orig_nr: LuaTable = module.raw_get("NumberRange")?;
    let orig_nr_new: LuaFunction = orig_nr.raw_get("new")?;
    nr_tbl.raw_set(
        "new",
        crate::lua::lua_fn(lua, move |_, (a, b): (f64, Option<f64>)| -> LuaResult<LuaValue> {
            match b {
                Some(b_val) if a > b_val => orig_nr_new.call((b_val, a)),
                Some(b_val) => orig_nr_new.call((a, b_val)),
                None => orig_nr_new.call((a,)),
            }
        })?,
    )?;
    nr_tbl.set_readonly(true);
    module.raw_set("NumberRange", nr_tbl)?;

    let real_enums: LuaValue = module.raw_get("Enum")?;
    let enum_wrapper: LuaValue = lua
        .load(
            r#"
            local real = ...
            local function makeEnum(enumName, items)
                local enumObj
                local byName = {}
                local list = {}
                local enumMt = {
                    __type = "Enum",
                    __tostring = function() return enumName end,
                    __index = function(_, k)
                        if k == "GetEnumItems" then
                            return function() return table.clone(list) end
                        end
                        if byName[k] ~= nil then return byName[k] end
                        error("'" .. tostring(k) .. "' is not a valid EnumItem of Enum." .. enumName, 2)
                    end,
                }
                enumObj = setmetatable({}, enumMt)
                for _, spec in ipairs(items) do
                    local itemName, itemVal = spec[1], spec[2]
                    local item = setmetatable({
                        Name = itemName,
                        Value = itemVal,
                        EnumType = enumObj,
                    }, {
                        __type = "EnumItem",
                        __tostring = function() return "Enum." .. enumName .. "." .. itemName end,
                    })
                    byName[itemName] = item
                    table.insert(list, item)
                end
                return enumObj
            end
            local extras = {
                GradientType = makeEnum("GradientType", { { "Linear", 0 }, { "Radial", 1 } }),
                GradientTileMode = makeEnum("GradientTileMode", { { "Clamp", 0 }, { "Repeat", 1 }, { "Mirror", 2 } }),
            }
            return setmetatable({}, {
                __type = "Enums",
                __tostring = function() return "Enum" end,
                __index = function(self, key)
                    if key == "GetEnums" then
                        local fn = function() return real:GetEnums() end
                        rawset(self, key, fn)
                        return fn
                    end
                    local ok, val = pcall(function() return real[key] end)
                    if ok then
                        rawset(self, key, val)
                        return val
                    end
                    if extras[key] ~= nil then
                        rawset(self, key, extras[key])
                        return extras[key]
                    end
                    error(val, 2)
                end,
            })
            "#,
        )
        .call(real_enums)?;
    module.raw_set("Enum", enum_wrapper)?;

    module.set_readonly(true);
    Ok(module)
}

// ---------------------------------------------------------------------------
// @lune/fs
// ---------------------------------------------------------------------------

fn copy_recursive(from: &Path, to: &Path, overwrite: bool) -> std::io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            let dest = to.join(entry.file_name());
            copy_recursive(&entry.path(), &dest, overwrite)?;
        }
        Ok(())
    } else {
        if !overwrite && to.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("destination already exists: {}", to.display()),
            ));
        }
        if let Some(parent) = to.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::copy(from, to)?;
        Ok(())
    }
}

fn overwrite_option(val: Option<LuaValue>) -> LuaResult<bool> {
    match val {
        None | Some(LuaValue::Nil) => Ok(false),
        Some(LuaValue::Boolean(b)) => Ok(b),
        Some(LuaValue::Table(t)) => Ok(t.get::<Option<bool>>("overwrite")?.unwrap_or(false)),
        _ => Ok(false),
    }
}

fn build_fs_module(lua: &Lua, rt: &Rc<LuneRuntime>) -> LuaResult<LuaTable> {
    let m = lua.create_table()?;

    let r = rt.clone();
    m.raw_set(
        "readFile",
        crate::lua::lua_fn(lua, move |lua, path: String| {
            let (norm, bytes) = r
                .cached_read_file(&path)
                .map_err(|e| LuaError::runtime(format!("fs.readFile({path}): {e}")))?;
            r.trace.borrow_mut().record_file_read_norm(&norm);
            lua.create_string(&*bytes)
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "readDir",
        crate::lua::lua_fn(lua, move |lua, path: String| {
            let (norm, entries) = r
                .cached_read_dir(&path)
                .map_err(|e| LuaError::runtime(format!("fs.readDir({path}): {e}")))?;
            r.trace.borrow_mut().record_dir_read_norm(&norm);
            let tbl = lua.create_table_with_capacity(entries.len(), 0)?;
            for (idx, name) in entries.iter().enumerate() {
                tbl.raw_set(idx + 1, name.as_str())?;
            }
            Ok(tbl)
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "isFile",
        crate::lua::lua_fn(lua, move |_, path: String| {
            let (norm, kind) = r.cached_stat(&path);
            r.trace.borrow_mut().record_stat_known(norm, kind);
            Ok(matches!(kind, CachedStat::File))
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "isDir",
        crate::lua::lua_fn(lua, move |_, path: String| {
            let (norm, kind) = r.cached_stat(&path);
            r.trace.borrow_mut().record_stat_known(norm, kind);
            Ok(matches!(kind, CachedStat::Dir))
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "metadata",
        crate::lua::lua_fn(lua, move |lua, path: String| {
            r.trace.borrow_mut().record_stat(&path);
            let tbl = lua.create_table()?;
            match fs::metadata(&path) {
                Ok(meta) => {
                    tbl.raw_set("exists", true)?;
                    let kind = if meta.is_file() {
                        LuaValue::String(lua.create_string("file")?)
                    } else if meta.is_dir() {
                        LuaValue::String(lua.create_string("dir")?)
                    } else {
                        LuaValue::Nil
                    };
                    tbl.raw_set("kind", kind)?;
                    let secs = |res: std::io::Result<std::time::SystemTime>| -> LuaValue {
                        res.ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map_or(LuaValue::Nil, |d| LuaValue::Number(d.as_secs_f64()))
                    };
                    tbl.raw_set("createdAt", secs(meta.created()))?;
                    tbl.raw_set("modifiedAt", secs(meta.modified()))?;
                    tbl.raw_set("accessedAt", secs(meta.accessed()))?;
                    let perms = lua.create_table()?;
                    perms.raw_set("readOnly", meta.permissions().readonly())?;
                    tbl.raw_set("permissions", perms)?;
                }
                Err(_) => {
                    tbl.raw_set("exists", false)?;
                    let perms = lua.create_table()?;
                    perms.raw_set("readOnly", false)?;
                    tbl.raw_set("permissions", perms)?;
                }
            }
            Ok(tbl)
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "writeFile",
        crate::lua::lua_fn(lua, move |_, (path, contents): (String, LuaValue)| {
            r.trace.borrow_mut().taint();
            if let Some(ref c) = r.fs_cache {
                c.invalidate_if_repo_path(&path);
            }
            let bytes = bytes_from_lua(&contents)?;
            fs::write(&path, &bytes).map_err(|e| LuaError::runtime(format!("fs.writeFile({path}): {e}")))?;
            Ok(())
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "writeDir",
        crate::lua::lua_fn(lua, move |_, path: String| {
            r.trace.borrow_mut().taint();
            if let Some(ref c) = r.fs_cache {
                c.invalidate_if_repo_path(&path);
            }
            fs::create_dir_all(&path).map_err(|e| LuaError::runtime(format!("fs.writeDir({path}): {e}")))?;
            Ok(())
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "removeFile",
        crate::lua::lua_fn(lua, move |_, path: String| {
            r.trace.borrow_mut().taint();
            if let Some(ref c) = r.fs_cache {
                c.invalidate_if_repo_path(&path);
            }
            fs::remove_file(&path).map_err(|e| LuaError::runtime(format!("fs.removeFile({path}): {e}")))?;
            Ok(())
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "removeDir",
        crate::lua::lua_fn(lua, move |_, path: String| {
            r.trace.borrow_mut().taint();
            if let Some(ref c) = r.fs_cache {
                c.invalidate_if_repo_path(&path);
            }
            fs::remove_dir_all(&path).map_err(|e| LuaError::runtime(format!("fs.removeDir({path}): {e}")))?;
            Ok(())
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "copy",
        crate::lua::lua_fn(lua, move |_, (from, to, opts): (String, String, Option<LuaValue>)| {
            r.trace.borrow_mut().taint();
            if let Some(ref c) = r.fs_cache {
                c.invalidate_if_repo_path(&to);
            }
            let overwrite = overwrite_option(opts)?;
            copy_recursive(Path::new(&from), Path::new(&to), overwrite)
                .map_err(|e| LuaError::runtime(format!("fs.copy({from} -> {to}): {e}")))?;
            Ok(())
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "move",
        crate::lua::lua_fn(lua, move |_, (from, to, opts): (String, String, Option<LuaValue>)| {
            r.trace.borrow_mut().taint();
            if let Some(ref c) = r.fs_cache {
                c.invalidate_if_repo_path(&from);
                c.invalidate_if_repo_path(&to);
            }
            let overwrite = overwrite_option(opts)?;
            let to_path = Path::new(&to);
            if !overwrite && to_path.exists() {
                return Err(LuaError::runtime(format!("fs.move: destination exists: {to}")));
            }
            if overwrite && to_path.exists() {
                if to_path.is_dir() {
                    let _ = fs::remove_dir_all(to_path);
                } else {
                    let _ = fs::remove_file(to_path);
                }
            }
            fs::rename(&from, &to).map_err(|e| LuaError::runtime(format!("fs.move({from} -> {to}): {e}")))?;
            Ok(())
        })?,
    )?;

    m.set_readonly(true);
    Ok(m)
}

// ---------------------------------------------------------------------------
// @lune/process
// ---------------------------------------------------------------------------

fn run_subprocess(
    rt: &LuneRuntime,
    program: &str,
    args: &[String],
    options: Option<&LuaTable>,
) -> LuaResult<CachedProcessOutput> {
    let git_spec = if options.is_none() {
        parse_immutable_git_show(program, args)
    } else {
        None
    };

    if let Some(ref spec) = git_spec {
        {
            let mut tr = rt.trace.borrow_mut();
            if tr.enabled {
                tr.git_shows.insert(spec.clone());
            }
        }
        let cache = GIT_SHOW_CACHE.get_or_init(|| RwLock::new(FxHashMap::default()));
        if let Ok(guard) = cache.read()
            && let Some(hit) = guard.get(spec)
        {
            return Ok(hit.clone());
        }
        let output = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| LuaError::runtime(format!("process({program}): {e}")))?;
        let cached = CachedProcessOutput {
            ok: output.status.success(),
            code: output.status.code().unwrap_or(1),
            stdout: Arc::from(output.stdout.as_slice()),
            stderr: Arc::from(output.stderr.as_slice()),
        };
        if let Ok(mut guard) = cache.write() {
            guard.insert(spec.clone(), cached.clone());
        }
        return Ok(cached);
    }

    rt.trace.borrow_mut().taint();
    let mut cmd = if let Some(opts) = options
        && let Ok(Some(shell)) = opts.get::<Option<LuaValue>>("shell")
    {
        let use_shell = match shell {
            LuaValue::Boolean(b) => b,
            LuaValue::String(_) => true,
            _ => false,
        };
        if use_shell {
            #[cfg(windows)]
            {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(program).args(args);
                c
            }
            #[cfg(not(windows))]
            {
                let mut c = Command::new("sh");
                let mut full = program.to_string();
                for a in args {
                    full.push(' ');
                    full.push_str(a);
                }
                c.arg("-c").arg(full);
                c
            }
        } else {
            let mut c = Command::new(program);
            c.args(args);
            c
        }
    } else {
        let mut c = Command::new(program);
        c.args(args);
        c
    };

    if let Some(opts) = options {
        if let Ok(Some(cwd)) = opts.get::<Option<String>>("cwd") {
            cmd.current_dir(cwd);
        }
        if let Ok(Some(env_tbl)) = opts.get::<Option<LuaTable>>("env") {
            for pair in env_tbl.pairs::<String, String>() {
                if let Ok((k, v)) = pair {
                    cmd.env(k, v);
                }
            }
        }
    }

    let output = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| LuaError::runtime(format!("process({program}): {e}")))?;
    Ok(CachedProcessOutput {
        ok: output.status.success(),
        code: output.status.code().unwrap_or(1),
        stdout: Arc::from(output.stdout.as_slice()),
        stderr: Arc::from(output.stderr.as_slice()),
    })
}

fn parse_string_args(val: Option<LuaValue>) -> LuaResult<Vec<String>> {
    match val {
        None | Some(LuaValue::Nil) => Ok(Vec::new()),
        Some(LuaValue::Table(t)) => {
            let mut out = Vec::new();
            for item in t.sequence_values::<String>() {
                out.push(item?);
            }
            Ok(out)
        }
        Some(other) => Err(LuaError::runtime(format!(
            "expected table of strings for process args, got {}",
            other.type_name()
        ))),
    }
}

fn build_process_module(lua: &Lua, rt: &Rc<LuneRuntime>) -> LuaResult<LuaTable> {
    let m = lua.create_table()?;
    let args_tbl = lua.create_table_with_capacity(rt.args.len(), 0)?;
    for (idx, arg) in rt.args.iter().enumerate() {
        args_tbl.raw_set(idx + 1, arg.as_str())?;
    }
    m.raw_set("args", args_tbl)?;
    m.raw_set("cwd", rt.cwd.as_str())?;
    m.raw_set(
        "os",
        if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        },
    )?;
    m.raw_set(
        "arch",
        if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            std::env::consts::ARCH
        },
    )?;
    m.raw_set(
        "endianness",
        if cfg!(target_endian = "little") { "little" } else { "big" },
    )?;

    // Traced env proxy
    let env_proxy = lua.create_table()?;
    let env_mt = lua.create_table()?;
    let r = rt.clone();
    env_mt.raw_set(
        "__index",
        crate::lua::lua_fn(lua, move |lua, (_, key): (LuaValue, LuaValue)| -> LuaResult<LuaValue> {
            match key {
                LuaValue::String(s) => {
                    let k = s.to_str()?;
                    let val = std::env::var(&*k).ok();
                    r.trace.borrow_mut().record_env(&k, val.clone());
                    match val {
                        Some(v) => Ok(LuaValue::String(lua.create_string(&v)?)),
                        None => Ok(LuaValue::Nil),
                    }
                }
                _ => {
                    r.trace.borrow_mut().taint();
                    Ok(LuaValue::Nil)
                }
            }
        })?,
    )?;
    let r = rt.clone();
    env_mt.raw_set(
        "__newindex",
        crate::lua::lua_fn(lua, move |_, (_, _key, _val): (LuaValue, LuaValue, LuaValue)| -> LuaResult<()> {
            r.trace.borrow_mut().taint();
            Ok(())
        })?,
    )?;
    let r = rt.clone();
    env_mt.raw_set(
        "__iter",
        crate::lua::lua_fn(lua, move |lua, _: LuaValue| -> LuaResult<LuaMultiValue> {
            r.trace.borrow_mut().taint();
            let snapshot = lua.create_table()?;
            for (k, v) in std::env::vars() {
                snapshot.raw_set(k, v)?;
            }
            let next_fn: LuaFunction = lua.globals().get("next")?;
            let mut out = LuaMultiValue::new();
            out.push_back(LuaValue::Function(next_fn));
            out.push_back(LuaValue::Table(snapshot));
            out.push_back(LuaValue::Nil);
            Ok(out)
        })?,
    )?;
    env_proxy.set_metatable(Some(env_mt))?;
    m.raw_set("env", env_proxy)?;

    let r = rt.clone();
    m.raw_set(
        "exit",
        crate::lua::lua_fn(lua, move |_, code: Option<i32>| -> LuaResult<()> {
            let code = code.unwrap_or(0);
            r.exit_code.set(Some(code));
            Err(LuaError::external(ExitSignal(code)))
        })?,
    )?;

    let r = rt.clone();
    let exec_fn = crate::lua::lua_fn(lua, 
        move |lua, (program, args, opts): (String, Option<LuaValue>, Option<LuaTable>)| {
            let argv = parse_string_args(args)?;
            let out = run_subprocess(&r, &program, &argv, opts.as_ref())?;
            let tbl = lua.create_table_with_capacity(0, 4)?;
            tbl.raw_set("ok", out.ok)?;
            tbl.raw_set("code", out.code)?;
            tbl.raw_set("stdout", lua.create_string(&*out.stdout)?)?;
            tbl.raw_set("stderr", lua.create_string(&*out.stderr)?)?;
            Ok(tbl)
        },
    )?;
    m.raw_set("exec", exec_fn.clone())?;
    m.raw_set("spawn", exec_fn)?;

    let r = rt.clone();
    m.raw_set(
        "create",
        crate::lua::lua_fn(lua, 
            move |lua, (program, args, opts): (String, Option<LuaValue>, Option<LuaTable>)| {
                let argv = parse_string_args(args)?;
                let out = run_subprocess(&r, &program, &argv, opts.as_ref())?;

                let make_reader = |bytes: Arc<[u8]>| -> LuaResult<LuaTable> {
                    let reader = lua.create_table()?;
                    let pos = Rc::new(Cell::new(0usize));
                    let b = bytes.clone();
                    let p = pos.clone();
                    reader.raw_set(
                        "readToEnd",
                        crate::lua::lua_fn(lua, move |lua, _: LuaMultiValue| {
                            let start = p.get().min(b.len());
                            p.set(b.len());
                            lua.create_string(&b[start..])
                        })?,
                    )?;
                    let b = bytes;
                    let p = pos;
                    reader.raw_set(
                        "read",
                        crate::lua::lua_fn(lua, move |lua, (_, size): (LuaValue, Option<usize>)| {
                            let start = p.get().min(b.len());
                            if start >= b.len() {
                                return Ok(LuaValue::Nil);
                            }
                            let end = match size {
                                Some(n) => (start + n).min(b.len()),
                                None => b.len(),
                            };
                            p.set(end);
                            Ok(LuaValue::String(lua.create_string(&b[start..end])?))
                        })?,
                    )?;
                    Ok(reader)
                };

                let child = lua.create_table()?;
                child.raw_set("stdout", make_reader(out.stdout)?)?;
                child.raw_set("stderr", make_reader(out.stderr)?)?;
                let ok = out.ok;
                let code = out.code;
                child.raw_set(
                    "status",
                    crate::lua::lua_fn(lua, move |lua, _: LuaMultiValue| {
                        let st = lua.create_table_with_capacity(0, 2)?;
                        st.raw_set("ok", ok)?;
                        st.raw_set("code", code)?;
                        Ok(st)
                    })?,
                )?;
                child.raw_set("kill", crate::lua::lua_fn(lua, |_, _: LuaMultiValue| Ok(()))?)?;
                Ok(child)
            },
        )?,
    )?;

    m.set_readonly(true);
    Ok(m)
}

// ---------------------------------------------------------------------------
// @lune/luau
// ---------------------------------------------------------------------------

struct EnvHelpers {
    globals_mt: LuaTable,
    merge_with_globals: LuaFunction,
    clone_table: LuaFunction,
}

fn create_env_helpers(lua: &Lua, globals_mt: LuaTable) -> LuaResult<EnvHelpers> {
    let factory: LuaFunction = lua
        .load(
            r#"
            local globals, globals_mt = ...
            local clone = table.clone
            local setmt = setmetatable
            local function merge_with_globals(env, parent)
                local merged = clone(globals)
                if parent ~= nil then
                    for k, v in parent do
                        merged[k] = v
                    end
                end
                for k, v in env do
                    merged[k] = v
                end
                return setmt(merged, globals_mt)
            end
            return merge_with_globals, clone
            "#,
        )
        .into_function()?;
    apply_codegen(lua, &factory, true);
    let (merge_with_globals, clone_table): (LuaFunction, LuaFunction) =
        factory.call((lua.globals(), globals_mt.clone()))?;
    Ok(EnvHelpers {
        globals_mt,
        merge_with_globals,
        clone_table,
    })
}

fn build_chunk_env(lua: &Lua, env: LuaTable, inject_globals: bool, helpers: &EnvHelpers) -> LuaResult<LuaTable> {
    let env_mt = env.metatable();
    let (has_fn_index, parent_tbl) = match env_mt.as_ref().and_then(|mt| mt.raw_get::<LuaValue>("__index").ok()) {
        Some(LuaValue::Function(_)) => (true, None),
        Some(LuaValue::Table(parent)) => (false, Some(parent)),
        _ => (false, None),
    };
    let has_custom_iter = env_mt
        .as_ref()
        .and_then(|mt| mt.raw_get::<LuaValue>("__iter").ok())
        .is_some_and(|v| !matches!(v, LuaValue::Nil));

    if inject_globals && has_fn_index {
        let merged = lua.create_table_with_capacity(0, 16)?;
        for pair in env.pairs::<LuaValue, LuaValue>() {
            let (k, v) = pair?;
            merged.raw_set(k, v)?;
        }
        let globals = lua.globals();
        let chain_mt = lua.create_table_with_capacity(0, 1)?;
        chain_mt.raw_set(
            "__index",
            crate::lua::lua_fn(lua, move |_, (_, key): (LuaValue, LuaValue)| -> LuaResult<LuaValue> {
                let v: LuaValue = env.get(key.clone())?;
                if !matches!(v, LuaValue::Nil) {
                    return Ok(v);
                }
                globals.raw_get(key)
            })?,
        )?;
        merged.set_metatable(Some(chain_mt))?;
        return Ok(merged);
    }

    if inject_globals && !has_custom_iter {
        let parent_val = parent_tbl.map_or(LuaValue::Nil, LuaValue::Table);
        return helpers.merge_with_globals.call((env, parent_val));
    }

    if inject_globals {
        let merged: LuaTable = helpers.clone_table.call(lua.globals())?;
        if let Some(parent) = parent_tbl {
            for pair in parent.pairs::<LuaValue, LuaValue>() {
                let (k, v) = pair?;
                merged.raw_set(k, v)?;
            }
        }
        for pair in env.pairs::<LuaValue, LuaValue>() {
            let (k, v) = pair?;
            merged.raw_set(k, v)?;
        }
        merged.set_metatable(Some(helpers.globals_mt.clone()))?;
        return Ok(merged);
    }

    let merged: LuaTable = helpers.clone_table.call(env)?;
    if let Some(mt) = env_mt {
        merged.set_metatable(Some(mt))?;
    }
    Ok(merged)
}

fn build_luau_module(lua: &Lua, helpers: Rc<EnvHelpers>, rt: &Rc<LuneRuntime>) -> LuaResult<LuaTable> {
    let m = lua.create_table()?;

    m.raw_set(
        "compile",
        crate::lua::lua_fn(lua, |lua, (source, options): (LuaString, Option<LuaTable>)| {
            let (mut opt, mut dbg, mut cov) = (1u8, 1u8, 0u8);
            if let Some(opts) = options {
                if let Some(v) = opts.get::<Option<u8>>("optimizationLevel")? {
                    opt = v;
                }
                if let Some(v) = opts.get::<Option<u8>>("debugLevel")? {
                    dbg = v;
                }
                if let Some(v) = opts.get::<Option<u8>>("coverageLevel")? {
                    cov = v;
                }
            }
            let bc = compile_cached(&source.as_bytes(), opt, dbg, cov)?;
            lua.create_string(&*bc)
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "load",
        crate::lua::lua_fn(lua, move |lua, (chunk, options): (LuaString, Option<LuaTable>)| {
            let bytes = chunk.as_bytes();
            let mut debug_name = String::from("luau.load(...)");
            let mut env: Option<LuaTable> = None;
            let mut inject_globals = true;
            let mut codegen_opt: Option<bool> = None;
            if let Some(opts) = options {
                if let Some(name) = opts.get::<Option<String>>("debugName")? {
                    debug_name = name;
                }
                if let Some(e) = opts.get::<Option<LuaTable>>("environment")? {
                    env = Some(e);
                }
                if let Some(ig) = opts.get::<Option<bool>>("injectGlobals")? {
                    inject_globals = ig;
                }
                if let Some(cg) = opts.get::<Option<bool>>("codegenEnabled")? {
                    codegen_opt = Some(cg);
                }
            }

            let compiled_buf;
            let bc: &[u8] = if is_luau_bytecode(&bytes) {
                &bytes
            } else {
                compiled_buf = compile_cached(&bytes, 1, 1, 0)?;
                &compiled_buf
            };

            // Tiered JIT budget: explicit codegenEnabled is always honored; when omitted, JIT-compile
            // long-lived modules (up to 64 per VM and at most 4 loads of any identical bytecode), while
            // letting repeated throwaway sandbox/mutation loads run directly in the interpreter.
            let should_codegen = match codegen_opt {
                Some(explicit) => explicit,
                None => {
                    let bc_hash = hash_bytes(bc, 0, 0, 0);
                    let mut counts = r.bytecode_loads.borrow_mut();
                    let count = counts.entry(bc_hash).or_insert(0);
                    *count = count.saturating_add(1);
                    let rem = r.jit_loads_remaining.get();
                    if *count <= 4 && rem > 0 {
                        r.jit_loads_remaining.set(rem - 1);
                        true
                    } else {
                        false
                    }
                }
            };

            let loaded = lua.load(bc).set_mode(mlua::chunk::ChunkMode::Binary).set_name(debug_name);
            let func = match env {
                Some(e) => {
                    let target = build_chunk_env(lua, e, inject_globals, &helpers)?;
                    loaded.set_environment(target).into_function()?
                }
                None if codegen_opt == Some(true) => loaded.into_function()?,
                None => {
                    // Fresh isolated environment inheriting globals so getfenv(fn).x = ... mutates
                    // only this chunk's env (matching Lune 0.10.5) without clobbering lua.globals().
                    let child_env = lua.create_table()?;
                    child_env.set_metatable(Some(helpers.globals_mt.clone()))?;
                    if should_codegen {
                        unsafe {
                            let _ = lua.exec_raw::<()>(child_env.clone(), |state| {
                                ffi::lua_setsafeenv(state, -1, 1);
                            });
                        }
                    }
                    loaded.set_environment(child_env).into_function()?
                }
            };
            apply_codegen(lua, &func, should_codegen);
            Ok(func)
        })?,
    )?;

    m.set_readonly(true);
    Ok(m)
}

// ---------------------------------------------------------------------------
// @lune/serde
// ---------------------------------------------------------------------------

fn json_to_lua(lua: &Lua, val: &Json) -> LuaResult<LuaValue> {
    match val {
        Json::Null => Ok(LuaValue::Nil),
        Json::Bool(b) => Ok(LuaValue::Boolean(*b)),
        Json::Number(n) => Ok(LuaValue::Number(n.as_f64().unwrap_or(0.0))),
        Json::String(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        Json::Array(arr) => {
            let tbl = lua.create_table_with_capacity(arr.len(), 0)?;
            for (i, item) in arr.iter().enumerate() {
                tbl.raw_set(i + 1, json_to_lua(lua, item)?)?;
            }
            Ok(LuaValue::Table(tbl))
        }
        Json::Object(map) => {
            let tbl = lua.create_table_with_capacity(0, map.len())?;
            for (k, v) in map {
                tbl.raw_set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(tbl))
        }
    }
}

fn lua_to_json(val: &LuaValue, depth: usize) -> LuaResult<Json> {
    if depth > 128 {
        return Err(LuaError::runtime("serde.encode: recursion limit exceeded"));
    }
    match val {
        LuaValue::Nil => Ok(Json::Null),
        LuaValue::Boolean(b) => Ok(Json::Bool(*b)),
        LuaValue::Integer(i) => Ok(Json::Number((*i).into())),
        LuaValue::Number(n) => {
            if !n.is_finite() {
                return Ok(Json::Null);
            }
            if n.fract() == 0.0 && *n >= (i64::MIN as f64) && *n <= (i64::MAX as f64) && *n != -0.0 {
                Ok(Json::Number((*n as i64).into()))
            } else {
                serde_json::Number::from_f64(*n)
                    .map(Json::Number)
                    .ok_or_else(|| LuaError::runtime("serde.encode: invalid float"))
            }
        }
        LuaValue::String(s) => Ok(Json::String(s.to_string_lossy())),
        LuaValue::Table(t) => {
            let len = t.raw_len();
            let mut count = 0usize;
            let mut all_array = true;
            for pair in t.pairs::<LuaValue, LuaValue>() {
                let (k, _) = pair?;
                count += 1;
                match k {
                    LuaValue::Integer(i) if i >= 1 && (i as usize) <= len => {}
                    LuaValue::Number(n) if n.fract() == 0.0 && n >= 1.0 && (n as usize) <= len => {}
                    _ => all_array = false,
                }
            }
            if count == 0 {
                return Ok(Json::Array(Vec::new()));
            }
            if all_array && count == len {
                let mut arr = Vec::with_capacity(len);
                for i in 1..=len {
                    let v: LuaValue = t.raw_get(i)?;
                    arr.push(lua_to_json(&v, depth + 1)?);
                }
                Ok(Json::Array(arr))
            } else {
                let mut map = JsonMap::with_capacity(count);
                for pair in t.pairs::<LuaValue, LuaValue>() {
                    let (k, v) = pair?;
                    let key_str = match k {
                        LuaValue::String(s) => s.to_string_lossy(),
                        LuaValue::Integer(i) => i.to_string(),
                        LuaValue::Number(n) => n.to_string(),
                        other => return Err(LuaError::runtime(format!("serde.encode: unsupported key type {}", other.type_name()))),
                    };
                    map.insert(key_str, lua_to_json(&v, depth + 1)?);
                }
                Ok(Json::Object(map))
            }
        }
        other => Err(LuaError::runtime(format!(
            "serde.encode: unsupported value type {}",
            other.type_name()
        ))),
    }
}

/// Minimal RFC 3174 SHA-1 for `@lune/serde`'s `hash("sha1", ...)`.
fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let bit_len = (data.len() as u64) * 8;
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    format!("{:08x}{:08x}{:08x}{:08x}{:08x}", h[0], h[1], h[2], h[3], h[4])
}

fn build_serde_module(lua: &Lua) -> LuaResult<LuaTable> {
    let m = lua.create_table()?;

    m.raw_set(
        "encode",
        crate::lua::lua_fn(lua, |_, (fmt, val, pretty): (String, LuaValue, Option<bool>)| match fmt.as_str() {
            "json" => {
                let json = lua_to_json(&val, 0)?;
                if pretty.unwrap_or(false) {
                    serde_json::to_string_pretty(&json).map_err(LuaError::external)
                } else {
                    serde_json::to_string(&json).map_err(LuaError::external)
                }
            }
            other => Err(LuaError::runtime(format!("serde.encode: unsupported format '{other}'"))),
        })?,
    )?;

    m.raw_set(
        "decode",
        crate::lua::lua_fn(lua, |lua, (fmt, text): (String, LuaValue)| {
            let bytes = bytes_from_lua(&text)?;
            let s = String::from_utf8(bytes).map_err(|e| LuaError::runtime(format!("serde.decode: invalid utf-8: {e}")))?;
            match fmt.as_str() {
                "json" => {
                    let parsed = crate::lune_json::decode(&s)
                        .map_err(|e| LuaError::runtime(format!("serde.decode(json): {e}")))?;
                    json_to_lua(lua, &parsed)
                }
                other => Err(LuaError::runtime(format!("serde.decode: unsupported format '{other}'"))),
            }
        })?,
    )?;

    m.raw_set(
        "compress",
        crate::lua::lua_fn(lua, |lua, (fmt, data, level): (String, LuaValue, Option<i32>)| {
            let bytes = bytes_from_lua(&data)?;
            match fmt.as_str() {
                "zstd" => {
                    let lvl = level.unwrap_or(0);
                    let compressed = zstd::bulk::compress(&bytes, lvl).map_err(LuaError::external)?;
                    lua.create_string(&compressed)
                }
                other => Err(LuaError::runtime(format!("serde.compress: unsupported format '{other}'"))),
            }
        })?,
    )?;

    m.raw_set(
        "decompress",
        crate::lua::lua_fn(lua, |lua, (fmt, data): (String, LuaValue)| {
            let bytes = bytes_from_lua(&data)?;
            match fmt.as_str() {
                "zstd" => {
                    let decompressed = zstd::stream::decode_all(&bytes[..]).map_err(LuaError::external)?;
                    lua.create_string(&decompressed)
                }
                other => Err(LuaError::runtime(format!("serde.decompress: unsupported format '{other}'"))),
            }
        })?,
    )?;

    m.raw_set(
        "hash",
        crate::lua::lua_fn(lua, |_, (algo, data): (String, LuaValue)| {
            let bytes = bytes_from_lua(&data)?;
            match algo.as_str() {
                "sha1" => Ok(sha1_hex(&bytes)),
                "sha256" => {
                    #[cfg(feature = "bundles")]
                    {
                        use sha2::Digest;
                        let digest = sha2::Sha256::digest(&bytes);
                        let mut hex = String::with_capacity(64);
                        for byte in digest {
                            use std::fmt::Write as _;
                            let _ = write!(&mut hex, "{byte:02x}");
                        }
                        Ok(hex)
                    }
                    #[cfg(not(feature = "bundles"))]
                    Err(LuaError::runtime("serde.hash(sha256) requires the bundles feature"))
                }
                other => Err(LuaError::runtime(format!("serde.hash: unsupported algorithm '{other}'"))),
            }
        })?,
    )?;

    m.set_readonly(true);
    Ok(m)
}

// ---------------------------------------------------------------------------
// @lune/stdio
// ---------------------------------------------------------------------------

fn build_stdio_module(lua: &Lua, rt: &Rc<LuneRuntime>) -> LuaResult<LuaTable> {
    let m = lua.create_table()?;
    let r = rt.clone();
    m.raw_set(
        "write",
        crate::lua::lua_fn(lua, move |_, text: LuaString| {
            r.output.write_stdout(&text.to_string_lossy());
            Ok(())
        })?,
    )?;
    let r = rt.clone();
    m.raw_set(
        "ewrite",
        crate::lua::lua_fn(lua, move |_, text: LuaString| {
            r.output.write_stderr(&text.to_string_lossy());
            Ok(())
        })?,
    )?;
    m.raw_set("color", crate::lua::lua_fn(lua, |_, _: String| Ok(""))?)?;
    m.raw_set("style", crate::lua::lua_fn(lua, |_, _: String| Ok(""))?)?;
    m.set_readonly(true);
    Ok(m)
}

// ---------------------------------------------------------------------------
// @lune/task
// ---------------------------------------------------------------------------

fn to_thread(lua: &Lua, val: LuaValue, fn_name: &str) -> LuaResult<LuaThread> {
    match val {
        LuaValue::Function(f) => lua.create_thread(f),
        LuaValue::Thread(t) => Ok(t),
        other => Err(LuaError::runtime(format!(
            "invalid argument #1 to 'task.{fn_name}' (function or thread expected, got {})",
            other.type_name()
        ))),
    }
}

fn build_task_module(lua: &Lua, rt: &Rc<LuneRuntime>) -> LuaResult<LuaTable> {
    let m = lua.create_table()?;

    let r = rt.clone();
    m.raw_set(
        "spawn",
        crate::lua::lua_fn(lua, move |lua, mut args: LuaMultiValue| {
            let Some(target) = args.pop_front() else {
                return Err(LuaError::runtime("missing argument #1 to 'task.spawn'"));
            };
            let thread = to_thread(lua, target, "spawn")?;
            if let Err(err) = thread.resume::<LuaMultiValue>(args) {
                if r.exit_code.get().is_some() {
                    return Err(err);
                }
                r.output.write_stderr(&format!("{err}\n"));
            }
            Ok(thread)
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "defer",
        crate::lua::lua_fn(lua, move |lua, mut args: LuaMultiValue| {
            let Some(target) = args.pop_front() else {
                return Err(LuaError::runtime("missing argument #1 to 'task.defer'"));
            };
            let thread = to_thread(lua, target, "defer")?;
            let rest: Vec<LuaValue> = args.into_iter().collect();
            r.tasks.borrow_mut().deferred.push_back((thread.clone(), rest));
            Ok(thread)
        })?,
    )?;

    let r = rt.clone();
    m.raw_set(
        "delay",
        crate::lua::lua_fn(lua, move |lua, mut args: LuaMultiValue| {
            let secs = match args.pop_front() {
                Some(LuaValue::Number(n)) => n.max(0.0),
                Some(LuaValue::Integer(n)) => (n as f64).max(0.0),
                Some(LuaValue::Nil) | None => 0.0,
                Some(other) => {
                    return Err(LuaError::runtime(format!(
                        "invalid argument #1 to 'task.delay' (number expected, got {})",
                        other.type_name()
                    )));
                }
            };
            let Some(target) = args.pop_front() else {
                return Err(LuaError::runtime("missing argument #2 to 'task.delay'"));
            };
            let thread = to_thread(lua, target, "delay")?;
            let now = Instant::now();
            let rest: Vec<LuaValue> = args.into_iter().collect();
            r.tasks.borrow_mut().timers.push(TimerEntry {
                deadline: now + Duration::from_secs_f64(secs),
                started: now,
                thread: thread.clone(),
                args: rest,
                is_wait: false,
            });
            Ok(thread)
        })?,
    )?;

    let r = rt.clone();
    let register_wait = crate::lua::lua_fn(lua, move |_, (thread, secs): (LuaThread, Option<f64>)| {
        let secs = secs.unwrap_or(0.0).max(0.0);
        let now = Instant::now();
        r.tasks.borrow_mut().timers.push(TimerEntry {
            deadline: now + Duration::from_secs_f64(secs),
            started: now,
            thread,
            args: Vec::new(),
            is_wait: true,
        });
        Ok(())
    })?;
    let wait_factory: LuaFunction = lua
        .load("local reg = ...; return function(secs) reg(coroutine.running(), secs); return coroutine.yield() end")
        .into_function()?;
    let wait_fn: LuaFunction = wait_factory.call(register_wait)?;
    m.raw_set("wait", wait_fn)?;

    let r = rt.clone();
    let cancel_mark = crate::lua::lua_fn(lua, move |_, thread: LuaThread| {
        r.tasks.borrow_mut().cancelled.insert(thread.to_pointer());
        Ok(())
    })?;
    let cancel_factory: LuaFunction = lua
        .load("local mark = ...; return function(th) mark(th); if coroutine.status(th) == 'suspended' then pcall(coroutine.close, th) end end")
        .into_function()?;
    let cancel_fn: LuaFunction = cancel_factory.call(cancel_mark)?;
    m.raw_set("cancel", cancel_fn)?;

    m.set_readonly(true);
    Ok(m)
}

fn drain_tasks(rt: &LuneRuntime) -> LuaResult<()> {
    loop {
        if rt.exit_code.get().is_some() {
            return Ok(());
        }
        let next_deferred = rt.tasks.borrow_mut().deferred.pop_front();
        if let Some((thread, args)) = next_deferred {
            let cancelled = rt.tasks.borrow_mut().cancelled.remove(&thread.to_pointer());
            if !cancelled && thread.status() == LuaThreadStatus::Resumable {
                let mv = LuaMultiValue::from_vec(args);
                if let Err(err) = thread.resume::<LuaMultiValue>(mv) {
                    if rt.exit_code.get().is_some() {
                        return Ok(());
                    }
                    rt.output.write_stderr(&format!("{err}\n"));
                }
            }
            continue;
        }
        let next_timer = {
            let mut q = rt.tasks.borrow_mut();
            if q.timers.is_empty() {
                None
            } else {
                let mut best = 0usize;
                for i in 1..q.timers.len() {
                    if q.timers[i].deadline < q.timers[best].deadline {
                        best = i;
                    }
                }
                Some(q.timers.swap_remove(best))
            }
        };
        let Some(timer) = next_timer else {
            break;
        };
        let cancelled = rt.tasks.borrow_mut().cancelled.remove(&timer.thread.to_pointer());
        if cancelled || timer.thread.status() != LuaThreadStatus::Resumable {
            continue;
        }
        let now = Instant::now();
        if timer.deadline > now {
            std::thread::sleep(timer.deadline - now);
        }
        let mv = if timer.is_wait {
            let mut v = LuaMultiValue::new();
            v.push_back(LuaValue::Number(timer.started.elapsed().as_secs_f64()));
            v
        } else {
            LuaMultiValue::from_vec(timer.args)
        };
        if let Err(err) = timer.thread.resume::<LuaMultiValue>(mv) {
            if rt.exit_code.get().is_some() {
                return Ok(());
            }
            rt.output.write_stderr(&format!("{err}\n"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Module loading & require resolution
// ---------------------------------------------------------------------------

fn sanitize_for_wrapper(source: &str) -> String {
    let mut out = source.replace("\r\n", "\n");
    let bytes = out.as_bytes();
    let mut pos = 0usize;
    let mut patches: Vec<usize> = Vec::new();
    while pos < bytes.len() {
        let line_end = bytes[pos..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |i| pos + i);
        let line = &out[pos..line_end];
        if line.starts_with("--!") {
            patches.push(pos + 2);
            pos = (line_end + 1).min(bytes.len());
        } else if line.trim().is_empty() || line.starts_with("--") {
            pos = (line_end + 1).min(bytes.len());
        } else {
            break;
        }
    }
    if !patches.is_empty() {
        // SAFETY: replacing single ASCII '!' with single ASCII ' ' preserves UTF-8 boundaries.
        let mut raw = out.into_bytes();
        for idx in patches {
            raw[idx] = b' ';
        }
        out = String::from_utf8(raw).expect("valid utf8");
    }
    if out.contains("export") {
        let mut replaced = String::with_capacity(out.len());
        let mut line_start = true;
        let bytes = out.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if line_start {
                let ws_start = i;
                while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                    i += 1;
                }
                replaced.push_str(&out[ws_start..i]);
                if out[i..].starts_with("export") {
                    let after_export = i + 6;
                    let mut j = after_export;
                    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                        j += 1;
                    }
                    if j > after_export
                        && out[j..].starts_with("type")
                        && bytes.get(j + 4).is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_')
                    {
                        replaced.push_str("      ");
                        i = after_export;
                    }
                }
                line_start = false;
            }
            if i < bytes.len() {
                let ch_end = ceil_char_boundary(&out, i + 1);
                if bytes[i] == b'\n' {
                    line_start = true;
                }
                replaced.push_str(&out[i..ch_end]);
                i = ch_end;
            }
        }
        out = replaced;
    }
    out
}

fn module_bases(module_path: &str) -> (String, String) {
    let file_name = module_path.rsplit('/').next().unwrap_or(module_path);
    let file_dir = dir_name(module_path);
    if file_name == "init.luau" || file_name == "init.lua" {
        (dir_name(&file_dir), file_dir)
    } else {
        let stem = module_path
            .strip_suffix(".luau")
            .or_else(|| module_path.strip_suffix(".lua"))
            .unwrap_or(module_path)
            .to_string();
        (file_dir, stem)
    }
}

fn resolve_relative_module(rt: &LuneRuntime, base_dir: &str, target: &str) -> LuaResult<String> {
    let base = join_norm(base_dir, target);
    let candidates: Vec<String> = if base.ends_with(".luau") || base.ends_with(".lua") {
        vec![base]
    } else {
        vec![
            format!("{base}.luau"),
            format!("{base}.lua"),
            format!("{base}/init.luau"),
            format!("{base}/init.lua"),
        ]
    };
    for candidate in &candidates {
        let (norm, kind) = rt.cached_stat(candidate);
        rt.trace.borrow_mut().record_stat_known(norm, kind);
        if matches!(kind, CachedStat::File) {
            return Ok(candidate.clone());
        }
    }
    Err(LuaError::runtime(format!(
        "could not resolve module '{target}' from '{base_dir}'"
    )))
}

fn make_require(lua: &Lua, rt: &Rc<LuneRuntime>, module_path: String) -> LuaResult<LuaFunction> {
    let (parent_dir, self_dir) = module_bases(&module_path);
    let rt = rt.clone();
    crate::lua::lua_fn(lua, move |lua, target: String| -> LuaResult<LuaValue> {
        if let Some(builtins) = rt.builtins.borrow().as_ref() {
            let hit: LuaValue = builtins.raw_get(target.as_str())?;
            if !matches!(hit, LuaValue::Nil) {
                return Ok(hit);
            }
        }
        let resolved = if target.starts_with("./") || target.starts_with("../") {
            resolve_relative_module(&rt, &parent_dir, &target)?
        } else if target == "@self" {
            resolve_relative_module(&rt, &self_dir, ".")?
        } else if let Some(rest) = target.strip_prefix("@self/") {
            resolve_relative_module(&rt, &self_dir, rest)?
        } else {
            return Err(LuaError::runtime(format!("unsupported module '{target}'")));
        };

        if let Some(key) = rt.module_cache.borrow().get(&resolved) {
            return lua.registry_value(key);
        }
        if !rt.loading.borrow_mut().insert(resolved.clone()) {
            return Err(LuaError::runtime(format!("cyclic require of '{resolved}'")));
        }
        let outcome = load_and_run_file(lua, &rt, &resolved, false);
        rt.loading.borrow_mut().remove(&resolved);
        let value = outcome?;
        let key = lua.create_registry_value(value.clone())?;
        rt.module_cache.borrow_mut().insert(resolved, key);
        Ok(value)
    })
}

fn load_file_chunk(lua: &Lua, rt: &Rc<LuneRuntime>, file_path: &str) -> LuaResult<LuaFunction> {
    let (norm, raw_bytes) = rt
        .cached_read_file(file_path)
        .map_err(|e| LuaError::runtime(format!("cannot read '{file_path}': {e}")))?;
    rt.trace.borrow_mut().record_file_read_norm(&norm);
    let source = String::from_utf8_lossy(&raw_bytes);
    let child_require = make_require(lua, rt, norm.clone())?;

    // Keep Luau safeenv + native JIT by binding `local require = ...` whenever getfenv/setfenv are absent.
    if !source.contains("getfenv") && !source.contains("setfenv") {
        let wrapped = format!(
            "local require=...;return function(...){}\nend",
            sanitize_for_wrapper(&source)
        );
        if let Ok(bc) = compile_cached(wrapped.as_bytes(), 2, 1, 0)
            && let Ok(factory) = lua
                .load(&*bc)
                .set_mode(mlua::chunk::ChunkMode::Binary)
                .set_name(format!("={norm}"))
                .into_function()
        {
            apply_codegen(lua, &factory, true);
            let chunk: LuaFunction = factory.call(child_require.clone())?;
            return Ok(chunk);
        }
    }

    let bc = compile_cached(source.as_bytes(), 2, 1, 0)?;
    let env = lua.create_table_with_capacity(0, 1)?;
    env.raw_set("require", child_require)?;
    let env_helpers = rt.env_helpers.borrow().clone().expect("env_helpers initialized");
    let merged = build_chunk_env(lua, env, true, &env_helpers)?;
    let chunk = lua
        .load(&*bc)
        .set_mode(mlua::chunk::ChunkMode::Binary)
        .set_name(format!("={norm}"))
        .set_environment(merged)
        .into_function()?;
    apply_codegen(lua, &chunk, true);
    Ok(chunk)
}

fn load_and_run_file(lua: &Lua, rt: &Rc<LuneRuntime>, file_path: &str, is_entry: bool) -> LuaResult<LuaValue> {
    let chunk = load_file_chunk(lua, rt, file_path)?;
    let call_args = if is_entry {
        let mut mv = LuaMultiValue::new();
        for a in &rt.args {
            mv.push_back(LuaValue::String(lua.create_string(a)?));
        }
        mv
    } else {
        LuaMultiValue::new()
    };
    if is_entry {
        let main_thread = lua.create_thread(chunk)?;
        let _ = main_thread.resume::<LuaMultiValue>(call_args)?;
        Ok(LuaValue::Nil)
    } else {
        chunk.call(call_args)
    }
}

// ---------------------------------------------------------------------------
// Single script execution
// ---------------------------------------------------------------------------

struct RunOutcome {
    code: i32,
    output: String,
    trace: Option<Json>,
}

fn format_print_args(lua: &Lua, args: &LuaMultiValue) -> String {
    let mut line = String::new();
    let tostring_fn: Option<LuaFunction> = lua.globals().get("tostring").ok();
    for (idx, val) in args.iter().enumerate() {
        if idx > 0 {
            line.push('\t');
        }
        if let Some(ref f) = tostring_fn
            && let Ok(s) = f.call::<LuaString>(val.clone())
        {
            line.push_str(&s.to_string_lossy());
        } else {
            line.push_str(&val.to_string().unwrap_or_else(|_| val.type_name().to_string()));
        }
    }
    line.push('\n');
    line
}

fn run_one(
    script: &str,
    args: Vec<String>,
    capture_output: bool,
    trace_enabled: bool,
    fs_cache: Option<Arc<BatchFsCache>>,
) -> RunOutcome {
    reset_internal_dom();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string());
    let rt = Rc::new(LuneRuntime {
        trace: RefCell::new(TraceState::new(trace_enabled)),
        output: OutputSink::new(capture_output),
        exit_code: Cell::new(None),
        tasks: RefCell::new(TaskQueue::new()),
        module_cache: RefCell::new(FxHashMap::default()),
        loading: RefCell::new(FxHashSet::default()),
        builtins: RefCell::new(None),
        env_helpers: RefCell::new(None),
        fs_cache,
        jit_loads_remaining: Cell::new(64),
        bytecode_loads: RefCell::new(FxHashMap::default()),
        args,
        cwd,
    });

    let lua = Lua::new();
    // Disable implicit JIT on every lua.load() call so throwaway luau.load chunks with custom
    // environments do not pay x86-64 codegen cost; entry files and codegenEnabled=true chunks
    // explicitly invoke apply_codegen().
    lua.enable_jit(false);
    unsafe {
        let _ = lua.exec_raw::<()>((), |state| {
            lua_setworkcounting(state, 0);
        });
    }

    let setup_result: LuaResult<()> = (|| {
        install_typeof(&lua)?;
        let globals = lua.globals();

        // Clear safeenv on any environment returned by getfenv(fn) so post-load monkey-patching
        // like `getfenv(mod.Fn).os = { clock = ... }` takes effect on GETIMPORT sites.
        let orig_getfenv: LuaFunction = globals.raw_get("getfenv")?;
        let wrapped_getfenv = crate::lua::lua_fn(&lua, move |lua, args: LuaMultiValue| -> LuaResult<LuaValue> {
            let env: LuaValue = orig_getfenv.call(args)?;
            if let LuaValue::Table(ref tbl) = env {
                unsafe {
                    let _ = lua.exec_raw::<()>(tbl.clone(), |state| {
                        ffi::lua_setsafeenv(state, -1, 0);
                    });
                }
            }
            Ok(env)
        })?;
        globals.raw_set("getfenv", wrapped_getfenv)?;

        let globals_mt = lua.create_table_with_capacity(0, 1)?;
        globals_mt.raw_set("__index", globals.clone())?;
        globals_mt.set_readonly(true);
        let env_helpers = Rc::new(create_env_helpers(&lua, globals_mt)?);
        *rt.env_helpers.borrow_mut() = Some(env_helpers.clone());

        let r = rt.clone();
        globals.raw_set(
            "print",
            crate::lua::lua_fn(&lua, move |lua, args: LuaMultiValue| {
                r.output.write_stdout(&format_print_args(lua, &args));
                Ok(())
            })?,
        )?;
        let r = rt.clone();
        globals.raw_set(
            "warn",
            crate::lua::lua_fn(&lua, move |lua, args: LuaMultiValue| {
                r.output.write_stderr(&format_print_args(lua, &args));
                Ok(())
            })?,
        )?;

        let builtins = lua.create_table()?;
        builtins.raw_set("@lune/fs", build_fs_module(&lua, &rt)?)?;
        builtins.raw_set("@lune/luau", build_luau_module(&lua, env_helpers, &rt)?)?;
        builtins.raw_set("@lune/process", build_process_module(&lua, &rt)?)?;
        builtins.raw_set("@lune/roblox", build_roblox_module(&lua)?)?;
        builtins.raw_set("@lune/serde", build_serde_module(&lua)?)?;
        builtins.raw_set("@lune/stdio", build_stdio_module(&lua, &rt)?)?;
        builtins.raw_set("@lune/task", build_task_module(&lua, &rt)?)?;
        builtins.set_readonly(true);
        *rt.builtins.borrow_mut() = Some(builtins);

        let entry_norm = normalize_path(script);
        let root_require = make_require(&lua, &rt, entry_norm)?;
        globals.raw_set("require", root_require)?;

        // Enable safeenv on the globals table so wrapped chunks run with VM fastcalls + native codegen.
        unsafe {
            lua.exec_raw::<()>((), |state| {
                ffi::lua_setsafeenv(state, ffi::LUA_GLOBALSINDEX, 1);
            })?;
        }
        Ok(())
    })();

    let code = match setup_result {
        Err(err) => {
            rt.output.write_stderr(&format!("ree lune setup error: {err}\n"));
            1
        }
        Ok(()) => {
            // Run entry script inside a coroutine so top-level `task.wait()` works cleanly.
            let run_res = (|| -> LuaResult<()> {
                let _ = load_and_run_file(&lua, &rt, script, true)?;
                drain_tasks(&rt)?;
                Ok(())
            })();
            match run_res {
                Ok(()) => rt.exit_code.get().unwrap_or(0),
                Err(err) => {
                    if let Some(code) = rt.exit_code.get() {
                        code
                    } else {
                        rt.output.write_stderr(&format!("{err}\n"));
                        1
                    }
                }
            }
        }
    };

    let trace = if trace_enabled {
        Some(rt.trace.borrow().to_json(code == 0))
    } else {
        None
    };
    let output = rt.output.take();
    // Drop the VM before resetting the thread-local DOM so Lua userdata referencing the DOM are gone first.
    drop(lua);
    reset_internal_dom();

    RunOutcome { code, output, trace }
}

// ---------------------------------------------------------------------------
// CLI entrypoints: `ree lune <script> [args...]` and `ree lune --batch <jobs.json>`
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LuneBatchSpec {
    workers: Option<usize>,
    jobs: Vec<LuneBatchJob>,
}

#[derive(Deserialize)]
struct LuneBatchJob {
    id: String,
    script: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    trace: bool,
}

pub fn cli(argv: &[String]) -> i32 {
    if argv.is_empty() || argv[0] == "--help" || argv[0] == "-h" {
        println!(
            "usage: ree lune <script.luau> [--] [args...]\n       ree lune --batch <jobs.json>"
        );
        return if argv.is_empty() { 2 } else { 0 };
    }
    if argv[0] == "--batch" || argv[0].starts_with("--batch=") {
        let path = if let Some(rest) = argv[0].strip_prefix("--batch=") {
            rest.to_string()
        } else if let Some(next) = argv.get(1) {
            next.clone()
        } else {
            eprintln!("ree lune --batch: <jobs.json> is required");
            return 2;
        };
        return batch_cli(&path);
    }

    let script = &argv[0];
    let mut script_args: Vec<String> = Vec::new();
    let rest = &argv[1..];
    let start = if rest.first().map(String::as_str) == Some("--") { 1 } else { 0 };
    script_args.extend_from_slice(&rest[start..]);

    let outcome = run_one(script, script_args, false, false, None);
    outcome.code
}

fn batch_cli(spec_path: &str) -> i32 {
    let text = match fs::read_to_string(spec_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("ree lune --batch: {spec_path}: {e}");
            return 2;
        }
    };
    let spec: LuneBatchSpec = match serde_json::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ree lune --batch: {spec_path}: {e}");
            return 2;
        }
    };

    let total = spec.jobs.len();
    if total == 0 {
        println!("{{\"results\":[]}}");
        return 0;
    }

    let workers = spec
        .workers
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
        .clamp(1, total);

    let queue = Arc::new(Mutex::new(spec.jobs.into_iter().enumerate().collect::<VecDeque<_>>()));
    let results: Arc<Mutex<Vec<Option<Json>>>> = Arc::new(Mutex::new(vec![None; total]));
    let fs_cache = Arc::new(BatchFsCache::new());
    let mut handles = Vec::with_capacity(workers);

    for worker_idx in 0..workers {
        let queue = queue.clone();
        let results = results.clone();
        let fs_cache = fs_cache.clone();
        let handle = std::thread::Builder::new()
            .name(format!("ree-lune-{worker_idx}"))
            .stack_size(64 << 20)
            .spawn(move || loop {
                let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                let Some((index, job)) = next else { break };
                let started = Instant::now();
                let outcome = run_one(&job.script, job.args, true, job.trace, Some(fs_cache.clone()));
                let wall = started.elapsed().as_secs_f64();
                let mut obj = json!({
                    "id": job.id,
                    "code": outcome.code,
                    "wallSeconds": wall,
                    "output": outcome.output,
                });
                if let Some(trace) = outcome.trace {
                    obj["trace"] = trace;
                }
                if let Ok(mut slots) = results.lock() {
                    slots[index] = Some(obj);
                }
            });
        match handle {
            Ok(h) => handles.push(h),
            Err(e) => {
                eprintln!("ree lune --batch: failed to spawn worker {worker_idx}: {e}");
                return 1;
            }
        }
    }

    for h in handles {
        if h.join().is_err() {
            eprintln!("ree lune --batch: worker panicked");
            return 1;
        }
    }

    let final_results: Vec<Json> = results
        .lock()
        .map(|mut slots| {
            slots
                .drain(..)
                .map(|slot| slot.unwrap_or_else(|| json!({ "code": 1, "output": "missing result" })))
                .collect()
        })
        .unwrap_or_default();
    let payload = serde_json::to_string(&json!({ "results": final_results })).unwrap_or_default();
    println!("{payload}");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_repo_bounds() {
        assert_eq!(normalize_path("tools\\foo/../bar.luau"), "tools/bar.luau");
        assert!(is_inside_repo("tools/bar.luau"));
        assert!(!is_inside_repo("../outside.luau"));
        assert!(!is_inside_repo("/etc/passwd"));
        assert!(!is_inside_repo("C:/Windows/win.ini"));
    }

    #[test]
    fn immutable_git_show_parser() {
        let a = vec!["-C".into(), ".".into(), "show".into(), "abcdef1:tools/a.luau".into()];
        assert_eq!(parse_immutable_git_show("git", &a).as_deref(), Some("abcdef1:tools/a.luau"));
        let b = vec!["show".into(), "HEAD:tools/a.luau".into()];
        assert_eq!(parse_immutable_git_show("git", &b), None);
    }

    #[test]
    fn sha1_matches_rfc3174() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn lune_runner_executes_native_vector3_and_serde() {
        let dir = std::env::temp_dir().join(format!("ree-lune-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let script = dir.join("smoke.luau");
        fs::write(
            &script,
            r#"
local roblox = require("@lune/roblox")
local serde = require("@lune/serde")
local luau = require("@lune/luau")
local task = require("@lune/task")
local process = require("@lune/process")

local v = roblox.Vector3.new(3, 4, 0)
assert(type(v) == "vector", "native vector expected")
assert(typeof(v) == "Vector3", "typeof Vector3 expected")
assert(v.Magnitude == 5, "Magnitude")
local cf = roblox.CFrame.new(1, 2, 3)
local pt = cf:PointToWorldSpace(v)
assert(typeof(pt) == "Vector3" and pt.X == 4 and pt.Y == 6 and pt.Z == 3, "CFrame interop")

local part = roblox.Instance.new("Part")
part.Position = v
assert(typeof(part.Position) == "Vector3" and part.Position.X == 3, "Instance Vector3 prop")

local enc = serde.encode("json", { a = 1, b = "hi" })
local dec = serde.decode("json", enc)
assert(dec.a == 1 and dec.b == "hi", "json roundtrip")
assert(serde.decompress("zstd", serde.compress("zstd", "hello world")) == "hello world", "zstd")

local fn = luau.load(luau.compile("return 40 + 2", { optimizationLevel = 2 }))
assert(fn() == 42, "luau load")

local ran = false
task.defer(function() ran = true end)
assert(process.args[1] == "hello", "args")
print("OK")
"#,
        )
        .unwrap();
        let out = run_one(&script.to_string_lossy(), vec!["hello".into()], true, false, None);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(out.code, 0, "stderr/stdout: {}", out.output);
        assert_eq!(out.output.trim(), "OK");
    }

    /// `ree lune`'s native Vector3 table keeps `vector_methods`'s distinct lowercase `lerp` C function
    /// (named `"lerp"`, not `rawequal` to `Lerp`, naming `'lerp'` in dotted errors) matching the engine.
    #[test]
    fn lune_runner_vector3_lowercase_lerp_matches_client() {
        let dir = std::env::temp_dir().join(format!("ree-lune-lerp-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let script = dir.join("lerp.luau");
        fs::write(
            &script,
            r#"
local roblox = require("@lune/roblox")
local a = roblox.Vector3.new(0, 2, 4)
local b = roblox.Vector3.new(10, 6, 8)
local m1 = a:lerp(b, 0.5)
local m2 = a:Lerp(b, 0.5)
assert(m1 == m2 and m1.X == 5 and m1.Y == 4 and m1.Z == 6, "lerp value")
assert(not rawequal(a.lerp, a.Lerp), "lerp must be distinct from Lerp")
local s1, l1, n1, ar1, v1 = debug.info(a.lerp, "slna")
local s2, l2, n2, ar2, v2 = debug.info(a.Lerp, "slna")
assert(s1 == "[C]" and l1 == -1 and n1 == "lerp" and ar1 == 0 and v1 == true, `bad lerp info: {s1}|{l1}|{n1}|{ar1}|{v1}`)
assert(s2 == "[C]" and l2 == -1 and n2 == "Lerp" and ar2 == 0 and v2 == true, `bad Lerp info: {s2}|{l2}|{n2}|{ar2}|{v2}`)
local ok1, err1 = pcall(a.lerp)
local ok2, err2 = pcall(a.lerp, a)
assert(not ok1 and string.find(tostring(err1), "to 'lerp'", 1, true), `bad err1: {err1}`)
assert(not ok2 and string.find(tostring(err2), "to 'lerp'", 1, true), `bad err2: {err2}`)
local c = roblox.Color3.new(0, 0.5, 1):lerp(roblox.Color3.new(1, 0.25, 0.25), 0.5)
assert(math.abs(c.R - 0.5) < 1e-6 and math.abs(c.G - 0.375) < 1e-6 and math.abs(c.B - 0.625) < 1e-6, "Color3:lerp")
print("OK")
"#,
        )
        .unwrap();
        let out = run_one(&script.to_string_lossy(), Vec::new(), true, false, None);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(out.code, 0, "stderr/stdout: {}", out.output);
        assert_eq!(out.output.trim(), "OK");
    }

    /// roblox.deserializeModel and deserializePlace read a Roblox archive (.rbx) as the file it holds, with the values
    /// kept outside its scene put back (a script's Source), and refuse a damaged one with the archive reader's error.
    #[test]
    fn lune_deserializes_archives_with_their_scripts_sources() {
        use rbx_dom_weak::{InstanceBuilder, WeakDom};
        let dir = std::env::temp_dir().join(format!("ree-lune-archive-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let source = format!("return \"{}\"
", "abcdefghij".repeat(40));
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let module = dom.insert(dom.root_ref(), InstanceBuilder::new("ModuleScript").with_name("Module").with_property("Source", source.clone()));
        let mut model = Vec::new();
        rbx_binary::to_writer(&mut model, &dom, &[module]).unwrap();
        let mut place = Vec::new();
        let storage = dom.insert(dom.root_ref(), InstanceBuilder::new("ReplicatedStorage"));
        dom.transfer_within(module, storage);
        rbx_binary::to_writer(&mut place, &dom, &[storage]).unwrap();
        let model = ror_archive::Builder::model(model).externalize(256).build().unwrap();
        let mut damaged = model.clone();
        let at = damaged.windows(10).position(|window| window == b"abcdefghij").unwrap();
        damaged[at] ^= 1;
        fs::write(dir.join("Model.rbx"), &model).unwrap();
        fs::write(dir.join("Damaged.rbx"), &damaged).unwrap();
        fs::write(dir.join("Place.rbx"), ror_archive::Builder::place(place).externalize(256).build().unwrap()).unwrap();
        let script = dir.join("archive.luau");
        fs::write(
            &script,
            r#"
local fs = require("@lune/fs")
local process = require("@lune/process")
local roblox = require("@lune/roblox")
local dir = process.args[1]
local model = roblox.deserializeModel(fs.readFile(dir .. "/Model.rbx"))
local place = roblox.deserializePlace(fs.readFile(dir .. "/Place.rbx"))
local damaged, message = pcall(roblox.deserializeModel, fs.readFile(dir .. "/Damaged.rbx"))
print(#model[1].Source, #place:GetService("ReplicatedStorage").Module.Source, damaged, string.find(tostring(message), "Roblox archive: invalid .rbx container", 1, true) ~= nil)
"#,
        )
        .unwrap();
        let out = run_one(&script.to_string_lossy(), vec![dir.to_string_lossy().into_owned()], true, false, None);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(out.code, 0, "stderr/stdout: {}", out.output);
        assert_eq!(out.output.trim(), format!("{0}	{0}	false	true", source.len()));
    }

    #[test]
    fn lune_runner_roblox_datatypes_instance_identity_and_env_isolation() {
        let dir = std::env::temp_dir().join(format!("ree-lune-compat-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let script = dir.join("compat.luau");
        fs::write(
            &script,
            r#"
local roblox = require("@lune/roblox")
local luau = require("@lune/luau")

-- Two-argument ColorSequence and NumberSequence constructors
local cs = roblox.ColorSequence.new(roblox.Color3.fromRGB(255, 127.5, 0), roblox.Color3.new(0, 1, 0))
assert(#cs.Keypoints == 2 and cs.Keypoints[1].Time == 0 and cs.Keypoints[2].Time == 1, "ColorSequence 2-arg")
local ns = roblox.NumberSequence.new(0.25, 0.75)
assert(#ns.Keypoints == 2 and ns.Keypoints[1].Value == 0.25 and ns.Keypoints[2].Value == 0.75, "NumberSequence 2-arg")

-- CFrame.fromMatrix with explicit nil 4th argument
local cf = roblox.CFrame.fromMatrix(roblox.Vector3.new(1, 2, 3), roblox.Vector3.new(1, 0, 0), roblox.Vector3.new(0, 1, 0), nil)
assert(cf.Position.X == 1 and cf.Position.Y == 2 and cf.Position.Z == 3, "CFrame.fromMatrix nil z")

-- Fractional UDim / UDim2 / Color3.fromRGB offsets truncate like Roblox
local u = roblox.UDim.new(0.5, 12.9)
assert(u.Offset == 12, "UDim fractional offset")
local u2 = roblox.UDim2.fromOffset(1.8, -3.7)
assert(u2.X.Offset == 1 and u2.Y.Offset == -3, "UDim2 fractional offset")

-- Instance userdata identity across table keys and property reads
local folder = roblox.Instance.new("Folder")
local child = roblox.Instance.new("Part")
child.Name = "Hero"
child.Parent = folder
local seen = { [child] = 123 }
assert(seen[folder:FindFirstChild("Hero")] == 123, "Instance table key identity")
assert(folder:FindFirstChild("Hero") == child, "Instance equality")

-- Built-in QueryDescendants and implementMethod override
local found = folder:QueryDescendants("Part")
assert(#found == 1 and found[1] == child, "default QueryDescendants")
roblox.implementMethod("Instance", "QueryDescendants", function()
    return { "override" }
end)
assert(folder:QueryDescendants("Part")[1] == "override", "overridden QueryDescendants")

-- Enum fallback (GradientType)
assert(roblox.Enum.GradientType ~= nil, "Enum.GradientType")

-- ReflectionDatabase wrapper matches @lune/roblox API
local db = roblox.getReflectionDatabase()
assert(type(db) == "userdata" and tostring(db) == "ReflectionDatabase", "db tostring")
assert(db == roblox.getReflectionDatabase(), "db equality")
assert(type(db.Version) == "string" and #db.Version > 0, "db.Version")
assert(#db:GetClassNames() > 100 and #db:GetEnumNames() > 100, "db lists")
local partCls = db:GetClass("Part")
assert(partCls ~= nil and partCls == db:FindClass("  part ") and tostring(partCls) == "ReflectionDatabaseClass(Part)", "db class")
assert(partCls.Name == "Part" and partCls.Superclass == "FormFactorPart", "db class fields")
local shapeProp = partCls.Properties.Shape
assert(shapeProp ~= nil and shapeProp.Name == "Shape" and shapeProp.Datatype == "Enum.PartType", "db prop")
assert(tostring(shapeProp) == "ReflectionDatabaseProperty(Part > Shape)", "db prop tostring")
local basePartCls = db:GetClass("BasePart")
assert(basePartCls ~= nil and basePartCls.Properties.Material.Datatype == "Enum.Material", "db basepart prop")
assert(partCls.DefaultProperties.Anchored == false, "db default bool")
assert(partCls.DefaultProperties.Shape == roblox.Enum.PartType.Block, "db default enum")
assert(partCls.DefaultProperties.CFrame ~= nil, "db default cframe")
local matEnum = db:GetEnum("Material")
assert(matEnum ~= nil and matEnum == db:FindEnum("material") and tostring(matEnum) == "ReflectionDatabaseEnum(Material)", "db enum")
assert(matEnum.Name == "Material" and type(matEnum.Items.Plastic) == "number", "db enum items")

-- Top-level coroutine.running() is non-nil
assert(coroutine.running() ~= nil, "top-level coroutine.running")

-- luau.load with env=nil isolates child env so getfenv(fn).os override works without leaking to caller
local bc = luau.compile("return function() return os.clock() end", { optimizationLevel = 2 })
local factory = luau.load(bc, { debugName = "clock_mod", codegenEnabled = false })
local getClock = factory()
getfenv(getClock).os = { clock = function() return 9876.5 end }
assert(getClock() == 9876.5, "getfenv(fn).os override must work")
assert(os.clock() ~= 9876.5, "child env mutation must not leak to globals")

print("COMPAT_OK")
"#,
        )
        .unwrap();
        let out = run_one(&script.to_string_lossy(), vec![], true, false, None);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(out.code, 0, "stderr/stdout: {}", out.output);
        assert_eq!(out.output.trim(), "COMPAT_OK");
    }

    #[test]
    fn batch_fs_cache_disables_on_in_repo_write() {
        let cache = BatchFsCache::new();
        assert!(!cache.disabled.load(Ordering::Acquire));
        cache.invalidate_if_repo_path("/outside/tmp.txt");
        assert!(!cache.disabled.load(Ordering::Acquire), "outside-repo write must not disable cache");
        cache.invalidate_if_repo_path("tools/some_file.luau");
        assert!(cache.disabled.load(Ordering::Acquire), "in-repo write must disable cache");
    }
}
