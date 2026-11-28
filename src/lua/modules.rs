//! ModuleScript loading (port of `runtime/engine/modules.luau`): instance requires, Roblox
//! require-by-string (./, ../, @self, @game, @<Service>), per-script environments, Roblox-style
//! caching and cyclic-require errors. The chunk itself runs in the prelude's `Require` (so a module
//! may yield while loading); these are its Rust halves.

use std::rc::Rc;

use mlua::{chunk::Compiler, prelude::*};
use rustc_hash::FxHashMap;

use super::{instance::materialize, instance_value, raw::instance_of_ud};
use crate::{Engine, InstanceId, K, NameId, dom::flags, value::Value};

#[path = "module_store.rs"]
pub(crate) mod store;

use store::StoredBytes;

#[derive(PartialEq, Eq)]
enum Status {
    Loading,
    Done,
    Error,
}

struct Record {
    status: Status,
    result: Option<LuaValue>,
    /// What a require of a module that failed raises (Status::Error)
    failure: &'static str,
    waiting: Vec<LuaThread>,
    /// The thread its chunk runs on (Status::Loading)
    owner: Option<LuaThread>,
}

/// What a require of a module whose chunk raised raises (measured: tools/parity/probes/require_errors.luau)
const LOAD_FAILED: &str = "Requested module experienced an error while loading";
/// What a require of a module whose chunk returned anything but one value raises, every later require too (measured, a 0.747
/// client and server: tools/parity/probes/engine_threads.luau)
const NOT_ONE_VALUE: &str = "Module code did not return exactly one value";
/// What a require of a module that is loading in the chain of loads now running raises
const RECURSIVE: &str = "Requested module was required recursively";
/// What a require of a path raises where no Luau function called require (`coroutine.create(require)`): measured, a 0.747 playing
/// client, tools/parity/probes/script_env.luau
const NO_LUAU_CALLER: &str = "require must be called by a Luau function in this context (e.g. function() require(...) end)";

impl Record {
    fn loading() -> Self {
        Record { status: Status::Loading, result: None, failure: LOAD_FAILED, waiting: Vec::new(), owner: None }
    }
}

/// The VM a script runs in: None for the main VM, else the key of its Actor ([`vm_key`]). An Actor runs its scripts in a VM of its own in
/// Studio: its own `_G` and its own copy of every module it requires.
pub type Vm = Option<u64>;

/// What the prelude passes for an Actor's VM: its instance id as a number.
pub fn vm_key(actor: InstanceId) -> u64 {
    (u64::from(actor.generation) << 32) | u64::from(actor.index)
}

#[derive(Default)]
pub struct ModuleRecords {
    records: FxHashMap<(Vm, InstanceId), Record>,
    /// The modules whose chunk a require is running now, outermost first: each was required from the chunk (or a thread that
    /// chunk runs) of the one before, without a yield between ([`require_run`])
    loading_now: Vec<(Vm, InstanceId)>,
    /// The parallel phases running (actors.rs)
    pub parallel: super::actors::Parallel,
    /// Source files of the modules required so far (`--dependencies`: what a run read)
    files: rustc_hash::FxHashSet<String>,
    /// Every source file read so far (required modules', JSON modules', Source reads)
    read: rustc_hash::FxHashSet<String>,
    /// Sources (and bytecode) read ahead on other threads ([`Prefetch`])
    prefetch: Option<std::sync::Arc<Prefetch>>,
    /// Compiled bytecode cached per script (keyed against the exact compiled source bytes) so repeated
    /// `debug.loadmodule` loads skip re-compiling unchanged source
    bytecode: FxHashMap<InstanceId, (Rc<[u8]>, StoredBytes)>,
    /// Compiled bytecode cached per script for `--native` `debug.loadmodule` (compiled with mutable sandbox globals)
    loadmodule_bytecode: FxHashMap<InstanceId, (Rc<[u8]>, StoredBytes)>,
    /// Loaded `debug.loadmodule` chunk templates (keyed against exact source bytes and full name) so repeated
    /// `debug.loadmodule` calls clone the immutable `Proto` and its native machine code instead of re-loading/re-JITing
    loadmodule_templates: FxHashMap<InstanceId, (Rc<[u8]>, String, LuaFunction)>,
    /// Loaded JSON `debug.loadmodule` inner closure templates keyed by `(path, full_name, bjson_ptr)` so repeated
    /// JSON `debug.loadmodule` calls clone the closure into a fresh environment instead of reloading bytecode/closures
    json_loadmodule_templates: FxHashMap<InstanceId, (Rc<str>, String, usize, LuaFunction)>,
    /// Single stateless no-op cleanup function returned by `debug.loadmodule`
    noop_cleanup: Option<LuaFunction>,
    /// Parsed JSON module values cached per engine so repeated `debug.loadmodule` loads skip re-reading and re-parsing
    json_cache: FxHashMap<String, std::sync::Arc<serde_json::Value>>,
    /// Compact binary JSON payloads cached per engine (pointing into mmap store or heap)
    bjson_cache: FxHashMap<String, StoredBytes>,
    /// Deferred native compilation ([`defer_codegen`]): module chunks loaded as bytecode only,
    /// compiled to native code later by [`compile_deferred`]
    deferred: Option<std::collections::VecDeque<LuaFunction>>,
    /// Module chunks load as bytecode only and stay interpreted ([`interpret_modules`])
    interpreted: bool,
    /// Every chunk of the VM runs in the interpreter, the engine's own included ([`interpret_all`]):
    /// loading a module leaves native code generation off
    interpret_all: bool,
    /// On-demand tiered JIT (`--native`): chunks load without eager codegen and hot Protos compile in place
    tiered_jit: bool,
    /// Module chunks this engine compiled from their text (not loaded from bytecode compiled ahead: [`Prefetch`],
    /// [`reuse_compiled_modules`]); a loopback run's second side compiles none of the modules both sides require
    pub text_compiles: u32,
    /// The chunk names a source text's later compiles take ([`ChunkKey`] -> its slot in the VM's table of first chunks, the name)
    chunk_names: FxHashMap<ChunkKey, (i64, Rc<str>)>,
}

/// A source text a VM compiled: its VM, its length and its 64-bit hash
type ChunkKey = (Vm, usize, u64);

impl ModuleRecords {
    /// A script got another source: the prefetched bytecode of the old one is stale.
    pub fn forget_bytecode(&mut self, script: InstanceId) {
        self.bytecode.remove(&script);
        self.loadmodule_bytecode.remove(&script);
        self.loadmodule_templates.remove(&script);
        self.json_loadmodule_templates.remove(&script);
    }
}

impl Drop for ModuleRecords {
    fn drop(&mut self) {
        store::flush_all();
    }
}

/// Module sources read ahead: the native app's engine worker starts one with the files the story
/// required the last time it was mounted (see `runtime.rs`), so their reads and their compilation
/// to bytecode (`luau_compile`, the same compiler options, no Lua state involved) run on other
/// threads while the engine starts and mounts. A require takes a file's bytes from here when they
/// are ready (else it reads the file as before); the bytecode is used only for the unpatched
/// source it was compiled from and a successful compile, so a chunk loads exactly as if it had
/// been compiled on the spot (compile errors take the ordinary path).
pub struct Prefetch {
    ready: std::sync::Mutex<std::collections::HashMap<String, (std::sync::Arc<[u8]>, Option<StoredBytes>)>>,
}

fn compile_std_cached(store: Option<&store::Store>, source: &[u8]) -> Option<StoredBytes> {
    let key = store::key_for(store::KIND_STD_BYTECODE, source);
    if let Some(found) = store.and_then(|s| s.get(&key)) {
        crate::profiler::caches::hit(crate::profiler::caches::Cache::Bytecode);
        return Some(found);
    }
    let compiled = compiler().compile(source).ok()?;
    let arc: std::sync::Arc<[u8]> = std::sync::Arc::from(compiled.into_boxed_slice());
    if let Some(store) = store {
        store.put(key, arc.clone());
    }
    Some(StoredBytes::owned(arc))
}

/// What [`parse_bjson`] says of a JSON module that runs the source Rojo writes for it, not its value
/// ([`crate::project::data_files::json_module`]: a string holding NUL, or deep nesting, which a client's compiler refuses)
pub(crate) const JSON_RUNS_SOURCE: &str = "the module runs the source Rojo writes for it";

fn parse_bjson_cached(store: Option<&store::Store>, source: &[u8]) -> Result<StoredBytes, String> {
    let key = store::key_for(store::KIND_BINARY_JSON, source);
    if let Some(found) = store.and_then(|s| s.get(&key)).filter(|b| store::validate_bjson(b)) {
        return Ok(found);
    }
    // (read as `rojo build` 7.6.1 reads a JSON module: lenient JSONC, strict JSON as serde_json reads it)
    let parsed = crate::project::data_files::json_module(source)?.ok_or_else(|| JSON_RUNS_SOURCE.to_string())?;
    let encoded = store::encode_bjson(&parsed);
    let arc: std::sync::Arc<[u8]> = std::sync::Arc::from(encoded.into_boxed_slice());
    if let Some(store) = store {
        store.put(key, arc.clone());
    }
    Ok(StoredBytes::owned(arc))
}

impl Prefetch {
    pub fn start(paths: Vec<String>) -> std::sync::Arc<Prefetch> {
        let prefetch = std::sync::Arc::new(Prefetch { ready: std::sync::Mutex::new(std::collections::HashMap::new()) });
        let shared = prefetch.clone();
        let disk = store::default_store();
        let _ = std::thread::Builder::new().name("rur-prefetch".into()).spawn(move || {
            use rayon::prelude::*;
            let Ok(pool) = rayon::ThreadPoolBuilder::new().num_threads(4).thread_name(|_| "rur-prefetch".into()).build() else { return };
            pool.install(|| {
                paths.par_iter().for_each(|path| {
                    let Ok(source) = std::fs::read(path) else { return };
                    if path.ends_with(".json") {
                        let _ = parse_bjson_cached(disk.as_deref(), &source);
                    }
                    let lua_source = path.ends_with(".lua") || path.ends_with(".luau");
                    let bytecode = if lua_source { compile_std_cached(disk.as_deref(), &source) } else { None };
                    let source_arc: std::sync::Arc<[u8]> = std::sync::Arc::from(source.into_boxed_slice());
                    if let Ok(mut ready) = shared.ready.lock() {
                        ready.insert(path.clone(), (source_arc, bytecode));
                    }
                });
            });
            if let Some(disk) = disk {
                disk.flush();
            }
        });
        prefetch
    }

    fn take(&self, path: &str) -> Option<(std::sync::Arc<[u8]>, Option<StoredBytes>)> {
        self.ready.lock().ok()?.remove(path)
    }
}

/// Uses `prefetch` for this engine's requires.
pub fn use_prefetch(engine: &Engine, prefetch: std::sync::Arc<Prefetch>) {
    engine.modules.borrow_mut().prefetch = Some(prefetch);
}

/// A file's bytes: prefetched when they are ready, from the process's compiled sources when it
/// keeps them ([`reuse_compiled_modules`]), else read now. When `compile_bytecode` is false, a
/// cache miss skips compiling standard `require` bytecode (used by JSON reads and `--native`
/// `debug.loadmodule`, which compiles with its own mutable-sandbox compiler).
fn read_file_opts(engine: &Engine, path: &str, compile_bytecode: bool) -> std::io::Result<(std::sync::Arc<[u8]>, Option<StoredBytes>)> {
    let prefetch = engine.modules.borrow().prefetch.clone();
    if let Some(taken) = prefetch.and_then(|prefetch| prefetch.take(path)) {
        crate::files::note(std::path::Path::new(path));
        return Ok(taken);
    }
    if let Some(compiled) = COMPILED.get() {
        let disk = store::for_engine(engine);
        return compiled.read_with_store(path, compile_bytecode, disk.as_deref());
    }
    crate::files::note(std::path::Path::new(path));
    let bytes = std::fs::read(path)?;
    let source: std::sync::Arc<[u8]> = std::sync::Arc::from(bytes.into_boxed_slice());
    let bytecode = if compile_bytecode && (path.ends_with(".lua") || path.ends_with(".luau")) {
        let disk = store::for_engine(engine);
        let key = store::key_for(store::KIND_STD_BYTECODE, &source);
        disk.as_deref().and_then(|s| s.get(&key))
    } else {
        None
    };
    Ok((source, bytecode))
}

#[cfg(test)]
fn read_file(engine: &Engine, path: &str) -> std::io::Result<(std::sync::Arc<[u8]>, Option<StoredBytes>)> {
    read_file_opts(engine, path, true)
}

/// Module sources and their bytecode kept by a process that runs many VMs (the render service,
/// serve.rs, or `ree run --batch`): a file whose size and modification time are unchanged is
/// neither read nor compiled again.
struct CompiledSources {
    entries: std::sync::RwLock<std::collections::HashMap<String, CompiledSource>>,
}

/// (stamp, source and optional standard bytecode, whether a lasting thread made it)
type CompiledSource = (crate::files::Stamp, std::sync::Arc<(std::sync::Arc<[u8]>, Option<StoredBytes>)>, bool);

thread_local! {
    /// A thread that ends with its job ([`precompile`]'s): what it keeps sits in a heap that outlives
    /// it, so a lasting thread copies it to its own on first use
    static TRANSIENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A kept source and bytecode, copied to this thread's long-lived heap (crate::alloc).
fn kept(source: &[u8], bytecode: Option<StoredBytes>) -> std::sync::Arc<(std::sync::Arc<[u8]>, Option<StoredBytes>)> {
    crate::alloc::long_lived(|| std::sync::Arc::new((std::sync::Arc::from(source), bytecode)))
}

static COMPILED: std::sync::OnceLock<CompiledSources> = std::sync::OnceLock::new();
/// Process-wide cache of `--native` `debug.loadmodule` bytecode (compiled with mutable sandbox globals)
/// so parallel `--batch` or `--service` workers compile each module from text once.
static LOADMODULE_COMPILED: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, (crate::files::Stamp, std::sync::Arc<(std::sync::Arc<[u8]>, StoredBytes)>)>>> = std::sync::OnceLock::new();
/// Process-wide cache of parsed JSON modules shared across `--batch` or `--service` workers.
static JSON_CACHE: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, (crate::files::Stamp, std::sync::Arc<serde_json::Value>)>>> = std::sync::OnceLock::new();
/// Process-wide cache of compact binary JSON payloads shared across `--batch` or `--service` workers.
static BJSON_CACHE: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, (crate::files::Stamp, StoredBytes)>>> = std::sync::OnceLock::new();
/// Process-wide cache of determinism-patched module bytecode (`.vide.values`), keyed by path and raw source bytes.
static PATCHED_BYTECODE: std::sync::OnceLock<
    std::sync::RwLock<FxHashMap<String, (std::sync::Arc<[u8]>, StoredBytes)>>,
> = std::sync::OnceLock::new();
/// Single-flight deduplication for cold bytecode compiles during `ree run --batch` so parallel
/// workers starting together compile each shared module once instead of N times simultaneously.
struct SingleFlight<V> {
    inflight: std::sync::Mutex<FxHashMap<String, std::sync::Arc<(std::sync::Mutex<Option<Result<V, String>>>, std::sync::Condvar)>>>,
}

impl<V: Clone> SingleFlight<V> {
    fn new() -> Self {
        Self { inflight: std::sync::Mutex::new(FxHashMap::default()) }
    }

    fn run(&self, key: &str, f: impl FnOnce() -> Result<V, String>) -> Result<V, String> {
        let (slot, owner) = {
            let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = map.get(key) {
                (existing.clone(), false)
            } else {
                let created = std::sync::Arc::new((std::sync::Mutex::new(None), std::sync::Condvar::new()));
                map.insert(key.to_string(), created.clone());
                (created, true)
            }
        };
        if owner {
            let res = f();
            {
                let (lock, cvar) = &*slot;
                *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(res.clone());
                cvar.notify_all();
            }
            if let Ok(mut map) = self.inflight.lock() {
                map.remove(key);
            }
            res
        } else {
            let (lock, cvar) = &*slot;
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            while guard.is_none() {
                guard = cvar.wait(guard).unwrap_or_else(|e| e.into_inner());
            }
            guard.as_ref().unwrap().clone()
        }
    }
}

static COMPILED_FLIGHT: std::sync::OnceLock<SingleFlight<std::sync::Arc<(std::sync::Arc<[u8]>, Option<StoredBytes>)>>> = std::sync::OnceLock::new();
static LOADMODULE_FLIGHT: std::sync::OnceLock<SingleFlight<StoredBytes>> = std::sync::OnceLock::new();
/// True during `ree run --batch`: files on disk stay constant for the batch, skipping redundant `stat()`
/// checks and duplicate precompile threads across parallel workers.
static BATCH_FROZEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_batch_frozen(frozen: bool) {
    BATCH_FROZEN.store(frozen, std::sync::atomic::Ordering::Relaxed);
}

pub fn is_batch_frozen() -> bool {
    BATCH_FROZEN.load(std::sync::atomic::Ordering::Relaxed)
}

impl CompiledSources {
    /// `path`'s source and optional standard bytecode, checked against the file now unless running
    /// inside a frozen one-shot `--batch` or a one-shot `--native` process that does not outlive its run.
    fn read_with_store(&self, path: &str, compile_bytecode: bool, disk: Option<&store::Store>) -> std::io::Result<(std::sync::Arc<[u8]>, Option<StoredBytes>)> {
        let keeping = keeping_between_runs();
        let one_shot = is_batch_frozen() || !keeping;
        let lasting = !TRANSIENT.with(std::cell::Cell::get);
        let lua_source = path.ends_with(".lua") || path.ends_with(".luau");
        if one_shot {
            crate::files::note(std::path::Path::new(path));
            if let Some((_, entry, _)) = self.entries.read().ok().and_then(|entries| entries.get(path).cloned()) {
                if !compile_bytecode || entry.1.is_some() || !lua_source {
                    crate::profiler::caches::hit(crate::profiler::caches::Cache::Bytecode);
                    return Ok((entry.0.clone(), entry.1.clone()));
                }
            }
            let flight = COMPILED_FLIGHT.get_or_init(SingleFlight::new);
            let flight_key = if compile_bytecode { format!("c:{path}") } else { format!("r:{path}") };
            let entry = flight
                .run(&flight_key, || {
                    if let Some((_, existing, _)) = self.entries.read().ok().and_then(|entries| entries.get(path).cloned()) {
                        if !compile_bytecode || existing.1.is_some() || !lua_source {
                            return Ok(existing);
                        }
                        let bytecode = compile_std_cached(disk, &existing.0);
                        let upgraded = if keeping {
                            kept(&existing.0, bytecode)
                        } else {
                            std::sync::Arc::new((existing.0.clone(), bytecode))
                        };
                        if let Ok(mut entries) = self.entries.write() {
                            let stamp = crate::files::Stamp { len: existing.0.len() as u64, modified: 0, directory: false };
                            if keeping {
                                crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, upgraded.clone(), lasting)));
                            } else {
                                entries.insert(path.to_string(), (stamp, upgraded.clone(), true));
                            }
                        }
                        return Ok(upgraded);
                    }
                    let source = std::fs::read(path).map_err(|e| e.to_string())?;
                    let bytecode = if compile_bytecode && lua_source { compile_std_cached(disk, &source) } else { None };
                    let entry = if keeping {
                        kept(&source, bytecode)
                    } else {
                        std::sync::Arc::new((std::sync::Arc::from(source.into_boxed_slice()), bytecode))
                    };
                    let stamp = crate::files::Stamp { len: entry.0.len() as u64, modified: 0, directory: false };
                    if let Ok(mut entries) = self.entries.write() {
                        if keeping {
                            crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, entry.clone(), lasting)));
                        } else {
                            entries.insert(path.to_string(), (stamp, entry.clone(), true));
                        }
                    }
                    Ok(entry)
                })
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            return Ok((entry.0.clone(), entry.1.clone()));
        }
        let file_path = std::path::Path::new(path);
        let stamp_opt = crate::files::stamp(file_path);
        crate::files::note_stamp(file_path, stamp_opt);
        let stamp = stamp_opt.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        if let Some((known, entry, home)) = self.entries.read().ok().and_then(|entries| entries.get(path).cloned()) {
            if known == stamp {
                if !compile_bytecode || entry.1.is_some() || !lua_source {
                    crate::profiler::caches::hit(crate::profiler::caches::Cache::Bytecode);
                    if lasting && !home {
                        let copy = kept(&entry.0, entry.1.clone());
                        if let Ok(mut entries) = self.entries.write() {
                            crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, copy, true)));
                        }
                    }
                    return Ok((entry.0.clone(), entry.1.clone()));
                }
                // Upgrade cached raw source with compiled standard bytecode.
                let bytecode = compile_std_cached(disk, &entry.0);
                let upgraded = kept(&entry.0, bytecode.clone());
                if let Ok(mut entries) = self.entries.write() {
                    crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, upgraded, lasting)));
                }
                return Ok((entry.0.clone(), bytecode));
            }
        }
        crate::profiler::caches::miss(crate::profiler::caches::Cache::Bytecode);
        let source = std::fs::read(path)?;
        let bytecode = if compile_bytecode && lua_source { compile_std_cached(disk, &source) } else { None };
        // (kept across runs: copied to the long-lived heap, away from the compiler's garbage)
        let entry = kept(&source, bytecode.clone());
        if let Ok(mut entries) = self.entries.write() {
            crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, entry.clone(), lasting)));
        }
        Ok((entry.0.clone(), bytecode))
    }

    fn read(&self, path: &str, compile_bytecode: bool) -> std::io::Result<(std::sync::Arc<[u8]>, Option<StoredBytes>)> {
        let disk = store::default_store();
        self.read_with_store(path, compile_bytecode, disk.as_deref())
    }
}

/// For a process that runs many VMs one after another (the render service): module sources and
/// bytecode are kept between VMs ([`CompiledSources`]), and engines created afterwards load chunks
/// without compiling them to native code (`luau_codegen_compile` costs a one-shot story more than
/// it gains; the interpreter gives the same results).
pub fn reuse_compiled_modules() {
    share_compiled_modules();
    KEEPING.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The process's compiled sources alone ([`CompiledSources`]), without what a service keeps between its runs (mount
/// listings, file stamps, model plans: [`keeping_between_runs`]): a loopback run's two sides compile each module once
pub fn share_compiled_modules() {
    let _ = COMPILED.set(CompiledSources { entries: std::sync::RwLock::new(Default::default()) });
    let _ = LOADMODULE_COMPILED.set(std::sync::RwLock::new(Default::default()));
    let _ = JSON_CACHE.set(std::sync::RwLock::new(Default::default()));
    let _ = BJSON_CACHE.set(std::sync::RwLock::new(Default::default()));
}

/// The process runs many VMs one after another and keeps what their mounts read ([`reuse_compiled_modules`])
static KEEPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether this process keeps mounts' listings, stamps and plans between runs (a service: [`reuse_compiled_modules`])
pub fn keeping_between_runs() -> bool {
    KEEPING.load(std::sync::atomic::Ordering::Relaxed)
}

/// Reads and compiles `paths` into the process's compiled sources on other threads (those not
/// kept yet), while the caller goes on: what a run is expected to require.
pub fn precompile(paths: Vec<String>) {
    let Some(compiled) = COMPILED.get() else { return };
    let missing: Vec<String> = {
        let Ok(entries) = compiled.entries.read() else { return };
        paths.into_iter().filter(|path| !entries.contains_key(path)).collect()
    };
    if missing.is_empty() {
        return;
    }
    let disk = store::default_store();
    let _ = std::thread::Builder::new().name("ree-precompile".into()).spawn(move || {
        use rayon::prelude::*;
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).thread_name(|_| "ree-precompile".into()).start_handler(|_| TRANSIENT.with(|transient| transient.set(true)));
        let Ok(pool) = pool.build() else { return };
        pool.install(|| {
            missing.par_iter().for_each(|path| {
                if let Ok((bytes, _)) = compiled.read_with_store(path, true, disk.as_deref()) {
                    if path.ends_with(".json") {
                        let _ = parse_bjson_cached(disk.as_deref(), &bytes);
                    }
                }
            });
        });
        if let Some(disk) = disk {
            disk.flush();
        }
    });
}

/// [`precompile`] on the thread pool, returning when every file is read and compiled (`ree serve
/// --warm`): how many were read now.
pub fn precompile_now(paths: &[String]) -> usize {
    use rayon::prelude::*;
    let Some(compiled) = COMPILED.get() else { return 0 };
    let missing: Vec<&String> = {
        let Ok(entries) = compiled.entries.read() else { return 0 };
        paths.iter().filter(|path| !entries.contains_key(*path)).collect()
    };
    let disk = store::default_store();
    let count = missing.par_iter().filter(|path| compiled.read_with_store(path, true, disk.as_deref()).is_ok()).count();
    if let Some(disk) = disk {
        disk.flush();
    }
    count
}

/// Drops the kept sources and bytecode of module files that changed or are gone (a changed file is
/// read and compiled again on its next require; its old entry would be kept for nothing).
pub fn forget_stale_modules() {
    if let Some(compiled) = COMPILED.get() {
        if let Ok(mut entries) = compiled.entries.write() {
            let before = entries.len();
            entries.retain(|path, (stamp, _, _)| crate::files::stamp(std::path::Path::new(path)) == Some(*stamp));
            crate::profiler::caches::evicted(crate::profiler::caches::Cache::Bytecode, (before - entries.len()) as u64);
        }
    }
    if let Some(loadmodule) = LOADMODULE_COMPILED.get() {
        if let Ok(mut entries) = loadmodule.write() {
            entries.retain(|path, (stamp, _)| crate::files::stamp(std::path::Path::new(path)) == Some(*stamp));
        }
    }
    if let Some(json) = JSON_CACHE.get() {
        if let Ok(mut entries) = json.write() {
            entries.retain(|path, (stamp, _)| crate::files::stamp(std::path::Path::new(path)) == Some(*stamp));
        }
    }
    if let Some(bjson) = BJSON_CACHE.get() {
        if let Ok(mut entries) = bjson.write() {
            entries.retain(|path, (stamp, _)| crate::files::stamp(std::path::Path::new(path)) == Some(*stamp));
        }
    }
    store::flush_all();
}

fn kept_loadmodule_bytecode(engine: &Engine, script: InstanceId, source: &[u8]) -> Option<StoredBytes> {
    let cache = LOADMODULE_COMPILED.get()?;
    let path = engine.state.borrow().dom.at(script).source_path().cloned()?;
    let entries = cache.read().ok()?;
    let (_, entry) = entries.get(&*path)?;
    (&*entry.0 == source).then(|| entry.1.clone())
}

fn store_loadmodule_bytecode(engine: &Engine, script: InstanceId, source: &[u8], bytecode: StoredBytes) {
    let Some(cache) = LOADMODULE_COMPILED.get() else { return };
    let Some(path) = engine.state.borrow().dom.at(script).source_path().cloned() else { return };
    let Some(stamp) = crate::files::stamp(std::path::Path::new(&*path)) else { return };
    let entry = crate::alloc::long_lived(|| std::sync::Arc::new((std::sync::Arc::from(source), bytecode)));
    if let Ok(mut entries) = cache.write() {
        crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, entry)));
    }
}

/// The process's bytecode of `script`'s file when it was compiled from exactly `source` ([`reuse_compiled_modules`]): a
/// loopback run's client copy of a module the server had read already carries its text, not a read of its own
fn kept_bytecode(engine: &Engine, script: InstanceId, source: &[u8]) -> Option<StoredBytes> {
    let compiled = COMPILED.get()?;
    let path = engine.state.borrow().dom.at(script).source_path().cloned()?;
    let entries = compiled.entries.read().ok()?;
    let (_, entry, _) = entries.get(&*path)?;
    (&*entry.0 == source).then(|| entry.1.clone()).flatten()
}

/// A file's bytes, through the process's compiled sources when it keeps them ([`reuse_compiled_modules`]): a loopback
/// run's harness, which both sides load, then compiles once ([`kept_bytecode`])
pub fn read_kept(path: &str) -> std::io::Result<Vec<u8>> {
    match COMPILED.get() {
        Some(compiled) => compiled.read(path, true).map(|(bytes, _)| bytes.to_vec()),
        None => std::fs::read(path),
    }
}

/// Whether [`reuse_compiled_modules`] is on.
pub fn reusing_compiled_modules() -> bool {
    COMPILED.get().is_some()
}

/// Defers native compilation of module chunks (the native app's engine worker, which compiles
/// them while it waits for commands): loading a chunk then skips `luau_codegen_compile`, the
/// largest part of a story's first mount, and its functions run in the interpreter until
/// [`compile_deferred`] compiles the chunk (native and interpreted execution behave the same;
/// functions entered later run natively).
pub fn defer_codegen(engine: &Engine) {
    engine.modules.borrow_mut().deferred.get_or_insert_with(Default::default);
}

/// Loads this engine's module chunks without native code generation, for good (`ree record` /
/// `ree run`: compiling a game's hundreds of modules to native code cost a recording ~1.2 s of its
/// ~1.5 s module loading and its frames ran no faster; the interpreter gives the same results).
pub fn interpret_modules(engine: &Engine) {
    engine.modules.borrow_mut().interpreted = true;
}

/// Loads every chunk of this VM, from now on, without native code generation: the engine's prelude,
/// services and Vector3 as well as module chunks (a render service makes a VM per render: compiling
/// those to native code at every VM start was most of the start; the interpreter gives the same
/// results). Call before the project mounts.
pub fn interpret_all(lua: &Lua, engine: &Engine) {
    {
        let mut modules = engine.modules.borrow_mut();
        modules.interpreted = true;
        modules.interpret_all = true;
    }
    crate::scheduler::start_cold_tracking(lua, engine);
    #[cfg(feature = "jit")]
    lua.enable_jit(false);
    #[cfg(not(feature = "jit"))]
    let _ = lua;
}

/// [`interpret_all`] for an engine whose VM has not started yet: Engine.Start loads the engine's own Luau interpreted too (a
/// loopback run's server: the engine's Luau is not charged to scripts, so it changes no output)
pub fn interpret_all_from_start(engine: &Engine) {
    let mut modules = engine.modules.borrow_mut();
    modules.interpreted = true;
    modules.interpret_all = true;
}

/// Marks this engine as using on-demand tiered JIT so chunk loads leave eager mlua JIT off.
pub fn use_tiered_jit(engine: &Engine) {
    engine.modules.borrow_mut().tiered_jit = true;
}

/// Whether [`interpret_all`] is on for this engine's VM
pub fn interprets_all(engine: &Engine) -> bool {
    engine.modules.borrow().interpret_all
}

/// Whether this engine's module chunks stay interpreted ([`interpret_modules`] or [`interpret_all`]): their Luau is what the work
/// clock counts and charges (a natively compiled chunk's instructions are not counted)
pub fn interprets_scripts(engine: &Engine) -> bool {
    engine.modules.borrow().interpreted
}

/// Compiles the oldest deferred chunk to native code; returns whether more are waiting.
pub fn compile_deferred(lua: &Lua, engine: &Engine) -> bool {
    let next = engine.modules.borrow_mut().deferred.as_mut().and_then(|queue| queue.pop_front());
    // (without feature `jit` there is no Luau native code generation: the VM interprets)
    #[cfg(not(feature = "jit"))]
    let _ = (lua, next);
    #[cfg(feature = "jit")]
    if let Some(function) = next {
        if unsafe { mlua::ffi::luau_codegen_supported() } != 0 {
            let _ = unsafe { lua.exec_raw::<()>(&function, |state| mlua::ffi::luau_codegen_compile(state, -1)) };
        }
    }
    engine.modules.borrow().deferred.as_ref().is_some_and(|queue| !queue.is_empty())
}

/// Every source file this engine has read so far (modules required, JSON modules, Source reads).
pub fn read_files(engine: &Engine) -> rustc_hash::FxHashSet<String> {
    engine.modules.borrow().read.clone()
}

/// Notes that the engine read source file `path` ([`read_files`]). File metadata noting for
/// prepared runs happens once on the actual stamp lookup (`files::note_stamp` / `files::note`).
fn note_read(engine: &Engine, path: &str) {
    engine.modules.borrow_mut().read.insert(path.to_string());
}

/// The source files of every module required so far, sorted. Also flushes any newly compiled
/// modules to the persistent cross-process store (one-shot CLI runs call this before exit).
pub fn required_files(engine: &Engine) -> Vec<String> {
    store::flush_all();
    let mut files: Vec<String> = engine.modules.borrow().files.iter().cloned().collect();
    crate::sort::ord(&mut files);
    files
}

/// Buffers small module-store deltas in memory between jobs of a long-lived batch worker until
/// enough entries or bytes accumulate or [`flush_module_store`] is called before exit.
pub fn set_deferred_store_flush(defer: bool) {
    store::set_deferred_flush(defer);
}

/// Flushes every pending entry to the persistent cross-process module store unconditionally.
pub fn flush_module_store() {
    store::flush_all_now();
}

/// Roblox compiles scripts at optimization level 2 (a UI library detects O2 through debug.info).
/// Scripts see the engine's `typeof` (Vector3 for native vectors, registered table types), so
/// the compiler must not fold or fast-call the builtin.
/// Vector3.new compiles as the VM's vector constructor (a fast call), as Roblox compiles it.
/// Constants fold in the float mode that flushes subnormal results, as Roblox's script loader folds them (lua/float_mode.rs): the
/// same in every run context, so the bytecode this compiler makes (kept by [`CompiledSources`], [`Prefetch`] and a loopback run's
/// sides) serves every engine.
/// A field read through one of [`FIELD_GLOBALS`] is read each time the code runs, as a client reads it.
pub fn compiler() -> Compiler {
    // (typeof is a builtin, as in Roblox, whose client runs a script's `typeof(x)` as a fast call: the VM's builtin names ree's
    // values as ree's typeof does, raw.rs typeof_function; measured in tools/cost-parity's env rows. The compiler leaves the
    // typeof of a constant vector to the VM, which names it Vector3, rather than folding it to "vector": Luau has done so
    // since LuauCompileTypeofFold's behaviour became its only one)
    FIELD_GLOBALS.iter().fold(
        Compiler::new().set_optimization_level(2).set_vector_ctor("Vector3.new").set_vector_type("Vector3").add_mutable_global(super::float_mode::COMPILE_FLUSH),
        |compiler, name| compiler.add_mutable_global(*name),
    )
}

/// The globals whose fields a script reads each time its code runs: the global is resolved when the script loads, as other
/// globals are, but `workspace.Gravity` or `game.Players.LocalPlayer` is no import resolved with it, so it reads the value of
/// the moment, not the one the chunk loaded with (patches/luau/11-compiler-g-written.patch, "@fields:"). Measured in a playing
/// client and a server (Studio 0.747, tools/parity/probes/field_imports.luau): a field of these written through a local reads
/// the new value through the global (Workspace.DistributedGameTime, which moves every frame, Gravity, CurrentCamera.CFrame, a
/// renamed child, script.Name, game.Name, shared.X), while a chain through Enum is resolved at load (`Enum.KeyCode.A` cost a
/// client 0.7 ns a read, a local's `.A` 26). `workspace.Gravity` cost a client 32.2 ns against 30.7 for the same read through a
/// local, `game.Workspace` 42.9 against 42.8: the global's import, then the field read each time.
pub const FIELD_GLOBALS: &[&str] = &["@fields:game", "@fields:Game", "@fields:workspace", "@fields:Workspace", "@fields:script", "@fields:shared"];

/// Compiler for `--native` `debug.loadmodule`: leaves sandbox-overridden globals mutable so `setfenv`
/// sandboxes can keep `safeenv` (fastcalls, inline `pairs`/`ipairs`, native codegen without deopts)
/// while still reading live overrides for `require`, `script`, timers, `_G`, and `shared`.
pub fn loadmodule_compiler() -> Compiler {
    compiler()
        .add_mutable_global("require")
        .add_mutable_global("script")
        .add_mutable_global("delay")
        .add_mutable_global("tick")
        .add_mutable_global("time")
        .add_mutable_global("DateTime")
        .add_mutable_global("os")
        .add_mutable_global("task")
        .add_mutable_global("_G")
        .add_mutable_global("shared")
        .add_mutable_global("print")
        .add_mutable_global("warn")
        .add_mutable_global("math")
}

/// The engine's own Luau (the prelude, services, vector3.luau), whose `Vector3` is Lune's.
pub fn engine_compiler() -> Compiler {
    Compiler::new().set_optimization_level(2).set_disabled_builtins(["typeof"]).add_mutable_global(super::float_mode::COMPILE_FLUSH)
}

/// How many times this process compiled a chunk of the engine's own Luau ([`load_engine_luau`]): once per chunk
static ENGINE_COMPILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The engine's own Luau chunks compiled so far, by the address of their (compiled-in) source
static ENGINE_BYTECODE: std::sync::Mutex<Vec<(usize, std::sync::Arc<[u8]>)>> = std::sync::Mutex::new(Vec::new());

/// Outcome published to callers waiting on an in-flight engine Luau compile.
#[derive(Clone)]
enum EngineBuild {
    Done(Result<std::sync::Arc<[u8]>, String>),
    /// The compiling worker unwound before finishing; waiters retry in their own thread.
    Panicked,
}

/// Single-flight coordination for engine Luau compilation so background warm-up and foreground load
/// never compile the same static chunk twice.
type EngineSlot = std::sync::Arc<(std::sync::Mutex<Option<EngineBuild>>, std::sync::Condvar)>;
static ENGINE_INFLIGHT: std::sync::Mutex<Vec<(usize, EngineSlot)>> = std::sync::Mutex::new(Vec::new());

fn remove_engine_inflight(key: usize) {
    let mut inflight = ENGINE_INFLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(pos) = inflight.iter().position(|(at, _)| *at == key) {
        inflight.swap_remove(pos);
    }
}

/// Cleans up `ENGINE_INFLIGHT` and wakes all waiters even if compilation unwinds.
struct EngineInflightGuard {
    key: usize,
    slot: EngineSlot,
    finished: bool,
}

impl EngineInflightGuard {
    fn complete(mut self, result: Result<std::sync::Arc<[u8]>, String>) -> Result<std::sync::Arc<[u8]>, String> {
        self.finished = true;
        remove_engine_inflight(self.key);
        let (lock, cvar) = &*self.slot;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(EngineBuild::Done(result.clone()));
        cvar.notify_all();
        result
    }
}

impl Drop for EngineInflightGuard {
    fn drop(&mut self) {
        if !self.finished {
            remove_engine_inflight(self.key);
            let (lock, cvar) = &*self.slot;
            *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(EngineBuild::Panicked);
            cvar.notify_all();
        }
    }
}

thread_local! {
    /// The compiles of [`ENGINE_COMPILES`] this thread made
    static ENGINE_COMPILES_HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many chunks of the engine's own Luau this process has compiled (each once: [`load_engine_luau`])
pub fn engine_compiles() -> usize {
    ENGINE_COMPILES.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many of those this thread compiled (a test's own count, whatever other threads start meanwhile)
pub fn engine_compiles_here() -> usize {
    ENGINE_COMPILES_HERE.with(std::cell::Cell::get)
}

fn ensure_engine_bytecode(source: &'static str, count_here: bool) -> Result<std::sync::Arc<[u8]>, String> {
    ensure_engine_bytecode_with(source, count_here, |src| engine_compiler().compile(src).map_err(|e| e.to_string()))
}

fn ensure_engine_bytecode_with(
    source: &'static str,
    count_here: bool,
    compile: impl Fn(&'static str) -> Result<Vec<u8>, String>,
) -> Result<std::sync::Arc<[u8]>, String> {
    let key = source.as_ptr() as usize;
    loop {
        if let Some(bytecode) = ENGINE_BYTECODE
            .lock()
            .ok()
            .and_then(|kept| kept.iter().find(|(at, _)| *at == key).map(|(_, bytecode)| bytecode.clone()))
        {
            return Ok(bytecode);
        }
        let (slot, owner) = {
            let mut inflight = ENGINE_INFLIGHT.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(bytecode) = ENGINE_BYTECODE
                .lock()
                .ok()
                .and_then(|kept| kept.iter().find(|(at, _)| *at == key).map(|(_, bytecode)| bytecode.clone()))
            {
                return Ok(bytecode);
            }
            if let Some((_, existing)) = inflight.iter().find(|(at, _)| *at == key) {
                (existing.clone(), false)
            } else {
                let created: EngineSlot = std::sync::Arc::new((std::sync::Mutex::new(None), std::sync::Condvar::new()));
                inflight.push((key, created.clone()));
                (created, true)
            }
        };
        if owner {
            let guard = EngineInflightGuard { key, slot, finished: false };
            let result = compile(source).map(|compiled| {
                ENGINE_COMPILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if count_here {
                    ENGINE_COMPILES_HERE.with(|count| count.set(count.get() + 1));
                }
                let bytecode: std::sync::Arc<[u8]> = crate::alloc::long_lived(|| std::sync::Arc::from(compiled.as_slice()));
                if let Ok(mut kept) = ENGINE_BYTECODE.lock()
                    && !kept.iter().any(|(at, _)| *at == key)
                {
                    crate::alloc::long_lived(|| kept.push((key, bytecode.clone())));
                }
                bytecode
            });
            return guard.complete(result);
        } else {
            let (lock, cvar) = &*slot;
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            let outcome = loop {
                if let Some(res) = guard.as_ref() {
                    break res.clone();
                }
                guard = cvar.wait(guard).unwrap_or_else(|e| e.into_inner());
            };
            match outcome {
                EngineBuild::Done(res) => return res,
                EngineBuild::Panicked => continue,
            }
        }
    }
}

/// Spawns parallel background compilation of the engine's static Luau chunks so they finish while
/// reflection loads instead of compiling sequentially on the main thread during `mount: start`.
pub(crate) fn warm_engine_luau(server: bool, loopback: bool) {
    if rayon::current_num_threads() <= 1 || rayon::current_thread_index().is_some() {
        return;
    }
    let mut chunks: [&'static str; 5] = [super::stubs::SERVICES, super::globals::PRELUDE, super::globals::VECTOR3, "", ""];
    let mut len = 3;
    if server {
        chunks[len] = super::stubs::SERVER;
        len += 1;
    }
    if loopback {
        chunks[len] = crate::loopback::remotes::LOOPBACK;
        len += 1;
    }
    let already_cached = |source: &'static str| {
        let key = source.as_ptr() as usize;
        ENGINE_BYTECODE.lock().ok().is_some_and(|kept| kept.iter().any(|(at, _)| *at == key))
    };
    for &source in &chunks[..len] {
        if !already_cached(source) {
            rayon::spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = ensure_engine_bytecode(source, false);
                }));
            });
        }
    }
}

/// A chunk of the engine's own Luau (the prelude, services.luau, server.luau, vector3.luau: a compiled-in `source`) as a
/// function named `name`, in `env` when given. Its bytecode is compiled once per process ([`engine_compiler`]) and every VM
/// loads those bytes, which are what its own compile would have made (mlua compiles a text chunk with its compiler and loads
/// the bytecode the same way), so a second engine in a process (a loopback run's peer, a service's next job) compiles none of
/// it: the 137 KB of engine Luau took 8.7-9.0 ms to compile and 0.34-0.37 ms to load (docs/engine.md, Log 2026-11-19). The
/// bytes stay in the long-lived heap (crate::alloc) for the process.
pub fn load_engine_luau(lua: &Lua, source: &'static str, name: &str, env: Option<LuaTable>) -> LuaResult<LuaFunction> {
    let bytecode = ensure_engine_bytecode(source, true).map_err(LuaError::runtime)?;
    let chunk = lua.load(&*bytecode).set_mode(mlua::chunk::ChunkMode::Binary).set_name(name);
    match env {
        Some(env) => chunk.set_environment(env).into_function(),
        None => chunk.into_function(),
    }
}

/// Determinism patch for vide's `values()` (modules.luau DeterministicSource): its output order
/// followed table iteration over address-keyed scopes; scopes are put in creation order.
pub fn deterministic_source<'a>(chunk_name: &str, source: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
    if !chunk_name.ends_with(".vide.values") {
        return source.into();
    }
    let find = |haystack: &[u8], needle: &[u8], from: usize| -> Option<usize> {
        haystack.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
    };
    let looped = b"for _, scope in scopes do";
    let create = b"scopes[v] = {";
    let (Some(loop_at), Some(create_at)) = (find(source, looped, 0), find(source, create, 0)) else {
        return source.into();
    };
    if find(source, looped, loop_at + 1).is_some() || find(source, create, create_at + 1).is_some() {
        return source.into();
    }
    let sorted: &[u8] = b"local __rur_sorted = {} for _, __rur_scope in scopes do table.insert(__rur_sorted, __rur_scope) end table.sort(__rur_sorted, function(a, b) return a.__rur_order < b.__rur_order end) for _, scope in __rur_sorted do";
    let mut patched = Vec::with_capacity(source.len() + 400);
    patched.extend_from_slice(&source[..loop_at]);
    patched.extend_from_slice(sorted);
    patched.extend_from_slice(&source[loop_at + looped.len()..]);
    let create_at = find(&patched, create, 0).unwrap();
    let mut result = Vec::with_capacity(patched.len() + 200);
    result.extend_from_slice(b"local __rur_order_count = 0 local function __rur_next_order() __rur_order_count += 1 return __rur_order_count end ");
    result.extend_from_slice(&patched[..create_at]);
    result.extend_from_slice(b"scopes[v] = { __rur_order = __rur_next_order(),");
    result.extend_from_slice(&patched[create_at + create.len()..]);
    result.into()
}

/// The environment a script of the main VM runs in ([`environment_in`]).
pub fn environment(lua: &Lua, engine: &Rc<Engine>, script: InstanceId) -> LuaResult<LuaTable> {
    environment_in(lua, engine, script, None)
}

/// Clones a loaded top-level chunk template into a fresh closure bound to `env`, reusing the
/// immutable `Proto` and its already-compiled native machine code without re-loading or re-JITing.
fn clone_with_env(lua: &Lua, template: &LuaFunction, env: &LuaTable) -> LuaResult<LuaFunction> {
    // SAFETY: `template` (slot 1) is a Luau function (0 upvalues) and `env` (slot 2) is a Lua table.
    // `lua_clonefunction` pushes a fresh LClosure sharing `Proto`, `lua_setfenv` binds `env`
    // without clearing `env->safeenv`, and slot 1 is replaced with the cloned closure as the sole result.
    unsafe {
        lua.exec_raw::<LuaFunction>((template, env), |state| {
            mlua::ffi::lua_rawcheckstack(state, 2);
            mlua::ffi::lua_clonefunction(state, -2);
            mlua::ffi::lua_pushvalue(state, -2);
            mlua::ffi::lua_setfenv(state, -2);
            mlua::ffi::lua_replace(state, -3);
            mlua::ffi::lua_pop(state, 1);
        })
    }
}

/// What a script environment's metatable answers getmetatable with (measured, a 0.747 playing client and a server:
/// tools/parity/probes/script_env.luau)
pub const LOCKED_METATABLE: &str = "The metatable is locked";

/// A metatable for the script environments of a VM: `__index` the globals, locked, and for an Actor's VM its key
/// ([`super::builtins::VM_FIELD`]), read only.
pub fn environment_metatable(lua: &Lua, shared: &LuaTable, vm: Vm) -> LuaResult<LuaTable> {
    let meta = lua.create_table_with_capacity(0, 3)?;
    meta.raw_set("__index", shared)?;
    meta.raw_set("__metatable", LOCKED_METATABLE)?;
    if let Some(key) = vm {
        meta.raw_set(super::builtins::VM_FIELD.to_str().expect("ascii"), key as f64)?;
    }
    meta.set_readonly(true);
    Ok(meta)
}

/// [`environment`] for a script of the VM `vm`, as a client makes one (measured, a 0.747 playing client and a server:
/// tools/parity/probes/script_env.luau): a table of its own holding `_G`, `script` and `shared` (`next` visits them in that order),
/// under a metatable the VM's environments share, whose `__index` is the globals (one `require` there for every script, which finds
/// the calling script: lua/builtins.rs require_function) and whose `__metatable` locks it: getmetatable answers "The metatable is
/// locked" and setmetatable raises "cannot change a protected metatable". An Actor's VM has a `_G` and a `shared` of its own (a
/// value one sets, the main VM's scripts do not see, nor it theirs).
pub fn environment_in(lua: &Lua, engine: &Rc<Engine>, script: InstanceId, vm: Vm) -> LuaResult<LuaTable> {
    // An interpreted script's environment is safe, as Roblox makes every script's: builtins take the VM's
    // fast calls and ipairs / pairs loops run inline (getfenv / setfenv end it, as there). Native code would
    // inline them with its own arithmetic (vector.magnitude in doubles), which Studio's interpreted scripts
    // do not, so a VM that compiles modules natively keeps the environment unsafe unless `--native` asks for
    // Roblox's native execution.
    let safe = engine.options.native || engine.modules.borrow().interpreted || cfg!(not(feature = "jit"));
    let (globals, meta, shared) = {
        let shared_globals = engine.lua.borrow().shared.clone().expect("shared globals");
        match vm {
            None => {
                let meta = engine.lua.borrow().env_meta.clone();
                let meta = match meta {
                    Some(meta) => meta,
                    None => {
                        let meta = environment_metatable(lua, &shared_globals, None)?;
                        engine.lua.borrow_mut().env_meta = Some(meta.clone());
                        meta
                    }
                };
                (shared_globals.raw_get::<LuaValue>("_G")?, meta, shared_globals.raw_get::<LuaValue>("shared")?)
            }
            Some(key) => {
                let found = engine.lua.borrow().vm_envs.get(&key).cloned();
                let (globals, shared, meta) = match found {
                    Some(found) => found,
                    None => {
                        let made = (lua.create_table()?, lua.create_table()?, environment_metatable(lua, &shared_globals, vm)?);
                        engine.lua.borrow_mut().vm_envs.insert(key, made.clone());
                        made
                    }
                };
                (LuaValue::Table(globals), meta, LuaValue::Table(shared))
            }
        }
    };
    let script_val = instance_value(lua, engine, script)?;
    // SAFETY: the four values are pushed as the arguments; the new table takes three of them as its fields and the fourth as its
    // metatable, and is left as the one result
    unsafe {
        lua.exec_raw::<LuaTable>((globals, script_val, shared, meta), move |state| {
            use mlua::ffi;
            ffi::lua_rawcheckstack(state, 2);
            // (stack: -4 _G, -3 script, -2 shared, -1 metatable)
            ffi::lua_createtable(state, 0, 3);
            ffi::lua_pushvalue(state, -5);
            ffi::lua_rawsetfield(state, -2, c"_G".as_ptr());
            ffi::lua_pushvalue(state, -4);
            ffi::lua_rawsetfield(state, -2, c"script".as_ptr());
            ffi::lua_pushvalue(state, -3);
            ffi::lua_rawsetfield(state, -2, c"shared".as_ptr());
            ffi::lua_pushvalue(state, -2);
            ffi::lua_setmetatable(state, -2);
            if safe {
                ffi::lua_setsafeenv(state, -1, 1);
            }
            ffi::lua_replace(state, -5);
            ffi::lua_pop(state, 3);
        })
    }
}

/// Modules.Compile: the chunk of a script's source in its own environment, named by its full name.
pub fn compile(lua: &Lua, engine: &Rc<Engine>, script: InstanceId, source: &[u8]) -> LuaResult<LuaFunction> {
    compile_in(lua, engine, script, source, None)
}

/// Pure straight-line data modules (no functions, loops, or `--!native`) execute every bytecode
/// instruction at most once, so skipping native x86-64 codegen avoids ~0.8 ms/module of compile
/// overhead with zero runtime loss.
fn should_jit_chunk(source: &[u8]) -> bool {
    let has = |needle: &[u8]| source.windows(needle.len()).any(|window| window == needle);
    has(b"function") || has(b"for") || has(b"while") || has(b"repeat") || has(b"--!native")
}

/// Compiles a ModuleScript for `debug.loadmodule`. Under `--native` it compiles with mutable sandbox globals, marks
/// the chunk for safe-sandbox `setfenv` preservation and keeps the loaded `Proto` (and its native machine code) per
/// `(script, source, full_name)`, so a repeated load clones the closure into a fresh environment instead of loading
/// and compiling it again. Without `--native` every load is a module's chunk loaded afresh in its own environment
/// ([`compile_in`]), as before: a shared template resolves its imports once, against an environment without the
/// script's `script` and `require`, so each of their reads became a run-time lookup the work clock charges
/// ([`crate::cost::LUAU_SLOW_IMPORT`]: 20,000 `script` reads cost a loaded module 214 us against a required one's 56).
pub fn compile_for_loadmodule(lua: &Lua, engine: &Rc<Engine>, script: InstanceId, source: &[u8]) -> LuaResult<LuaFunction> {
    if !engine.options.native {
        return compile_in_with(lua, engine, script, source, None, None, false);
    }
    let env = environment_in(lua, engine, script, None)?;
    let name = engine.state.borrow().dom.full_name(&engine.reflection, script);
    if crate::profiler::active() {
        let path = engine.state.borrow().dom.at(script).source_path().map(|path| path.to_string());
        crate::profiler::source_loaded(&name, path.as_deref(), &std::rc::Rc::from(source));
    }
    let source = deterministic_source(&name, source);
    let cached_template = engine
        .modules
        .borrow()
        .loadmodule_templates
        .get(&script)
        .and_then(|(cached_src, cached_name, template)| {
            (cached_src.as_ref() == &*source && cached_name == &name).then(|| template.clone())
        });
    if let Some(template) = cached_template {
        return clone_with_env(lua, &template, &env);
    }

    let cached = engine
        .modules
        .borrow()
        .loadmodule_bytecode
        .get(&script)
        .and_then(|(cached_src, cached_bc)| (cached_src.as_ref() == &*source).then(|| cached_bc.clone()));
    let bytecode = match cached {
        Some(bc) => Some(bc),
        None => {
            let from_kept = matches!(&source, std::borrow::Cow::Borrowed(_))
                .then(|| kept_loadmodule_bytecode(engine, script, &source))
                .flatten();
            if let Some(bc) = from_kept {
                engine.modules.borrow_mut().loadmodule_bytecode.insert(script, (Rc::from(&*source), bc.clone()));
                Some(bc)
            } else {
                let disk = store::for_engine(engine);
                let disk_key = store::key_for(store::KIND_LOADMODULE_BYTECODE, &*source);
                if let Some(bc) = disk.as_deref().and_then(|s| s.get(&disk_key)) {
                    crate::profiler::caches::hit(crate::profiler::caches::Cache::Bytecode);
                    if matches!(&source, std::borrow::Cow::Borrowed(_)) {
                        store_loadmodule_bytecode(engine, script, &source, bc.clone());
                    }
                    engine.modules.borrow_mut().loadmodule_bytecode.insert(script, (Rc::from(&*source), bc.clone()));
                    Some(bc)
                } else if is_batch_frozen() && matches!(&source, std::borrow::Cow::Borrowed(_)) {
                    let key = engine
                        .state
                        .borrow()
                        .dom
                        .at(script)
                        .source_path()
                        .map(|p| p.to_string());
                    let compiled = if let Some(path_key) = key {
                        let flight = LOADMODULE_FLIGHT.get_or_init(SingleFlight::new);
                        flight
                            .run(&path_key, || {
                                if let Some(existing) = kept_loadmodule_bytecode(engine, script, &source) {
                                    return Ok(existing);
                                }
                                let bc = loadmodule_compiler().compile(&*source).map_err(|e| e.to_string())?;
                                let arc: std::sync::Arc<[u8]> = std::sync::Arc::from(bc.into_boxed_slice());
                                if let Some(s) = disk.as_deref() {
                                    s.put(disk_key, arc.clone());
                                }
                                let stored = StoredBytes::owned(arc);
                                store_loadmodule_bytecode(engine, script, &source, stored.clone());
                                Ok(stored)
                            })
                            .ok()
                    } else {
                        loadmodule_compiler().compile(&*source).ok().map(StoredBytes::from_vec)
                    };
                    match compiled {
                        Some(bc) => {
                            engine.modules.borrow_mut().loadmodule_bytecode.insert(script, (Rc::from(&*source), bc.clone()));
                            Some(bc)
                        }
                        None => None,
                    }
                } else {
                    engine.modules.borrow_mut().text_compiles += 1;
                    match loadmodule_compiler().compile(&*source) {
                        Ok(bc) => {
                            let arc: std::sync::Arc<[u8]> = std::sync::Arc::from(bc.into_boxed_slice());
                            if let Some(s) = disk.as_deref() {
                                s.put(disk_key, arc.clone());
                            }
                            let stored = StoredBytes::owned(arc);
                            if matches!(&source, std::borrow::Cow::Borrowed(_)) {
                                store_loadmodule_bytecode(engine, script, &source, stored.clone());
                            }
                            engine.modules.borrow_mut().loadmodule_bytecode.insert(script, (Rc::from(&*source), stored.clone()));
                            Some(stored)
                        }
                        Err(_) => None,
                    }
                }
            }
        }
    };

    // The template is loaded against the shared globals (no `script`/`require` keys: the loadmodule compiler keeps
    // those and the other sandbox-overridden globals mutable, so they are never imports) so `luau_load` resolves the
    // library imports without baking per-script globals or keeping any caller's environment in the template cache.
    let template_env = engine.lua.borrow().shared.clone().expect("shared globals");
    let tiered = engine.modules.borrow().tiered_jit;
    let skip_jit = !tiered && !should_jit_chunk(&source);
    let load_template = |chunk: LuaChunk| {
        if skip_jit {
            #[cfg(feature = "jit")]
            let was_interpreted = engine.modules.borrow().interpret_all;
            #[cfg(feature = "jit")]
            lua.enable_jit(false);
            let loaded = crate::timing::time("modules: compile + load", || {
                chunk.set_name(format!("={name}")).set_environment(template_env.clone()).into_function()
            });
            #[cfg(feature = "jit")]
            if !was_interpreted {
                lua.enable_jit(true);
            }
            loaded
        } else {
            load(lua, engine, chunk.set_name(format!("={name}")).set_environment(template_env.clone()))
        }
    };
    let text = || lua.load(&*source).set_mode(mlua::chunk::ChunkMode::Text).set_compiler(loadmodule_compiler());
    let template = match &bytecode {
        Some(bytecode) => match load_template(lua.load(&**bytecode).set_mode(mlua::chunk::ChunkMode::Binary)) {
            Ok(template) => template,
            Err(_) => {
                // (bytecode kept from an earlier compile that does not load: see compile_in)
                forget_unloadable(engine, script);
                load_template(text())?
            }
        },
        None => load_template(text())?,
    };
    crate::cost::set_safe_sandbox(lua, &template)?;
    engine
        .modules
        .borrow_mut()
        .loadmodule_templates
        .insert(script, (Rc::from(&*source), name, template.clone()));
    clone_with_env(lua, &template, &env)
}

/// [`compile`] for a script of the VM `vm`.
pub fn compile_in(lua: &Lua, engine: &Rc<Engine>, script: InstanceId, source: &[u8], vm: Vm) -> LuaResult<LuaFunction> {
    compile_in_with(lua, engine, script, source, None, vm, true)
}

/// The hash a source text's compiles are matched by ([`ChunkKey`])
fn chunk_key(vm: Vm, source: &[u8]) -> ChunkKey {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(source);
    (vm, source.len(), hasher.finish())
}

/// The name of the first chunk the VM compiled from this source text, while that chunk lives (see [`compile_in_with`])
fn first_chunk_name(engine: &Engine, key: &ChunkKey) -> Option<Rc<str>> {
    let (slot, name) = engine.modules.borrow().chunk_names.get(key).cloned()?;
    let firsts = engine.lua.borrow().chunk_firsts.clone()?;
    matches!(firsts.raw_get::<LuaValue>(slot), Ok(LuaValue::Function(_))).then_some(name)
}

/// Keeps `chunk`, named `name`, as the first chunk of its source text (held weakly: [`super::LuaRefs::chunk_firsts`])
fn remember_first_chunk(lua: &Lua, engine: &Engine, key: ChunkKey, chunk: &LuaFunction, name: &str) -> LuaResult<()> {
    let firsts = engine.lua.borrow().chunk_firsts.clone();
    let firsts = match firsts {
        Some(firsts) => firsts,
        None => {
            let firsts = lua.create_table()?;
            let meta = lua.create_table()?;
            meta.raw_set("__mode", "v")?;
            firsts.set_metatable(Some(meta));
            engine.lua.borrow_mut().chunk_firsts = Some(firsts.clone());
            firsts
        }
    };
    let mut modules = engine.modules.borrow_mut();
    let slot = match modules.chunk_names.get(&key) {
        Some(&(slot, _)) => slot,
        None => modules.chunk_names.len() as i64 + 1,
    };
    modules.chunk_names.insert(key, (slot, Rc::from(name)));
    drop(modules);
    firsts.raw_set(slot, chunk)
}

/// A script's chunk (`share`: a script's or a module's, not `debug.loadmodule`'s), named by the script's full name as Roblox names
/// it, unless the VM compiled the same source text before and that first chunk still lives: then it carries that chunk's name
/// (its errors, tracebacks and `debug.info(f, "s")` read it). A client keeps the first chunk it compiles from each text, holding
/// it only weakly, and compiles a later script of that text from it: so a module or a script (LocalScript, Script) whose source
/// equals an earlier one's is named as the earlier one was when it compiled, until a collection finds that chunk unreferenced
/// (a chunk the earlier script's code still holds, or that is still running, keeps it), however many other texts compile between.
/// A source that does not compile names no chunk; `loadstring`'s and `debug.loadmodule`'s chunks are their own, and an Actor's VM
/// keeps its own. Measured, a 0.747 playing client and a server: tools/parity/probes/chunknames.luau, require_errors.luau.
fn compile_in_with(lua: &Lua, engine: &Rc<Engine>, script: InstanceId, raw_source: &[u8], precompiled: Option<StoredBytes>, vm: Vm, share: bool) -> LuaResult<LuaFunction> {
    let key = share.then(|| chunk_key(vm, raw_source));
    let first = key.as_ref().and_then(|key| first_chunk_name(engine, key));
    let (name, source_path) = {
        let state = engine.state.borrow();
        (state.dom.full_name(&engine.reflection, script), state.dom.at(script).source_path().cloned())
    };
    let chunk_name = format!("={}", first.as_deref().unwrap_or(&name));
    let compiled = compile_named(lua, engine, script, raw_source, precompiled, vm, &name, source_path, &chunk_name);
    if let (Some(key), None, Ok(chunk)) = (key, &first, &compiled) {
        remember_first_chunk(lua, engine, key, chunk, &name)?;
    }
    compiled
}

/// [`compile_in_with`]'s chunk of the script `name` (its own full name), named `chunk_name`
#[allow(clippy::too_many_arguments)]
fn compile_named(
    lua: &Lua,
    engine: &Rc<Engine>,
    script: InstanceId,
    raw_source: &[u8],
    precompiled: Option<StoredBytes>,
    vm: Vm,
    name: &str,
    source_path: Option<Rc<str>>,
    chunk_name: &str,
) -> LuaResult<LuaFunction> {
    let env = environment_in(lua, engine, script, vm)?;
    // (a profile's annotated listing reads the chunk's source: profiler.rs)
    if crate::profiler::active() {
        crate::profiler::source_loaded(&name, source_path.as_deref(), &std::rc::Rc::from(raw_source));
    }
    let is_vide_values = name.ends_with(".vide.values");
    if is_vide_values {
        if let Some(ref path) = source_path {
            if let Some(bc) = PATCHED_BYTECODE
                .get()
                .and_then(|map| map.read().ok())
                .and_then(|map| map.get(&**path).and_then(|(cached_raw, bc)| (&**cached_raw == raw_source).then(|| bc.clone())))
            {
                let chunk = lua.load(&*bc).set_mode(mlua::chunk::ChunkMode::Binary).set_name(chunk_name).set_environment(env.clone());
                if let Ok(function) = load(lua, engine, chunk) {
                    return Ok(function);
                }
                forget_unloadable(engine, script);
            }
        }
    }
    let source = if is_vide_values { deterministic_source(&name, raw_source) } else { std::borrow::Cow::Borrowed(raw_source) };
    let patched = matches!(&source, std::borrow::Cow::Owned(_));
    let bytecode = if !patched && precompiled.is_some() {
        precompiled
    } else {
        let cached = engine
            .modules
            .borrow()
            .bytecode
            .get(&script)
            .and_then(|(cached_src, cached_bc)| (cached_src.as_ref() == &*source).then(|| cached_bc.clone()));
        match cached {
            Some(bc) => Some(bc),
            None => {
                let from_kept = (!patched).then(|| kept_bytecode(engine, script, &source)).flatten();
                if let Some(bc) = from_kept {
                    engine.modules.borrow_mut().bytecode.insert(script, (Rc::from(&*source), bc.clone()));
                    Some(bc)
                } else {
                    let disk = store::for_engine(engine);
                    let disk_key = store::key_for(store::KIND_STD_BYTECODE, &*source);
                    if let Some(bc) = disk.as_deref().and_then(|s| s.get(&disk_key)) {
                        crate::profiler::caches::hit(crate::profiler::caches::Cache::Bytecode);
                        if patched && keeping_between_runs() {
                            if let Some(ref path) = source_path {
                                let cache = PATCHED_BYTECODE.get_or_init(|| crate::alloc::long_lived(Default::default));
                                if let Ok(mut map) = cache.write() {
                                    let raw_arc: std::sync::Arc<[u8]> = crate::alloc::long_lived(|| std::sync::Arc::from(raw_source));
                                    crate::alloc::long_lived(|| map.insert(path.to_string(), (raw_arc, bc.clone())));
                                }
                            }
                        }
                        engine.modules.borrow_mut().bytecode.insert(script, (Rc::from(&*source), bc.clone()));
                        Some(bc)
                    } else {
                        engine.modules.borrow_mut().text_compiles += 1;
                        match compiler().compile(&*source) {
                            Ok(bc) => {
                                let arc: std::sync::Arc<[u8]> = std::sync::Arc::from(bc.into_boxed_slice());
                                if let Some(s) = disk.as_deref() {
                                    s.put(disk_key, arc.clone());
                                }
                                let stored = StoredBytes::owned(arc);
                                if patched && keeping_between_runs() {
                                    if let Some(ref path) = source_path {
                                        let cache = PATCHED_BYTECODE.get_or_init(|| crate::alloc::long_lived(Default::default));
                                        if let Ok(mut map) = cache.write() {
                                            let raw_arc: std::sync::Arc<[u8]> = crate::alloc::long_lived(|| std::sync::Arc::from(raw_source));
                                            crate::alloc::long_lived(|| map.insert(path.to_string(), (raw_arc, stored.clone())));
                                        }
                                    }
                                }
                                engine.modules.borrow_mut().bytecode.insert(script, (Rc::from(&*source), stored.clone()));
                                Some(stored)
                            }
                            Err(_) => None,
                        }
                    }
                }
            }
        }
    };
    // A script's chunk is its full name as Roblox names it: error positions, tracebacks and debug.info(f, "s") read
    // `ReplicatedStorage.Module:3:`, not Luau's `[string "ReplicatedStorage.Module"]:3:` for an unmarked name
    // (measured in a playing client, Studio 0.744: tools/parity/probes/chunknames.luau). The source is always text, as
    // Roblox compiles it: left to guess, mlua takes text whose first byte is below 9 for bytecode, and a zero first byte
    // then fails to load with an error whose text is only the chunk name.
    let text = || lua.load(&*source).set_mode(mlua::chunk::ChunkMode::Text).set_compiler(compiler()).set_name(chunk_name).set_environment(env.clone());
    match &bytecode {
        Some(bytecode) => {
            let chunk = lua.load(&**bytecode).set_mode(mlua::chunk::ChunkMode::Binary).set_name(chunk_name).set_environment(env.clone());
            load(lua, engine, chunk).or_else(|_| {
                // Bytecode compiled earlier (kept by this process, or read from the module store) that does not load is
                // not the script's error: compile the source again, so a script fails only for its own reasons
                forget_unloadable(engine, script);
                engine.modules.borrow_mut().text_compiles += 1;
                load(lua, engine, text())
            })
        }
        None => load(lua, engine, text()),
    }
}

/// Test hook: keeps `bytecode` as `script`'s compiled source, as a prefetch, the process's kept sources or the module store
/// hand bytecode over.
#[cfg(test)]
pub(crate) fn keep_bytecode(engine: &Engine, script: InstanceId, bytecode: Vec<u8>) {
    let source = source_rc(engine, script).expect("a source");
    engine.modules.borrow_mut().bytecode.insert(script, (source, StoredBytes::from_vec(bytecode)));
}

/// Drops the bytecode kept for `script` (this engine's and the process's, by its file) after it failed to load, so it is
/// compiled from its source again.
fn forget_unloadable(engine: &Engine, script: InstanceId) {
    engine.modules.borrow_mut().forget_bytecode(script);
    let Some(path) = engine.state.borrow().dom.at(script).source_path().cloned() else { return };
    if let Some(compiled) = COMPILED.get() {
        if let Ok(mut entries) = compiled.entries.write() {
            entries.remove(&*path);
        }
    }
    if let Some(compiled) = LOADMODULE_COMPILED.get() {
        if let Ok(mut entries) = compiled.write() {
            entries.remove(&*path);
        }
    }
    if let Some(patched) = PATCHED_BYTECODE.get() {
        if let Ok(mut entries) = patched.write() {
            entries.remove(&*path);
        }
    }
}

/// Loads a script's chunk as this engine's VM runs scripts: interpreted, or compiled to native code now or later
/// ([`defer_codegen`], [`interpret_modules`]). A module's chunk ([`compile_in`]) and loadstring's (loading.rs).
pub fn load(lua: &Lua, engine: &Rc<Engine>, chunk: LuaChunk) -> LuaResult<LuaFunction> {
    let (deferred, interpreted, interpret_all, tiered_jit) = {
        let modules = engine.modules.borrow();
        (modules.deferred.is_some(), modules.interpreted, modules.interpret_all, modules.tiered_jit)
    };
    if tiered_jit || (!deferred && !interpreted) {
        return crate::timing::time("modules: compile + load", || chunk.into_function());
    }
    #[cfg(feature = "jit")]
    lua.enable_jit(false);
    let function = crate::timing::time("modules: compile + load", || chunk.into_function());
    #[cfg(feature = "jit")]
    if !interpret_all {
        lua.enable_jit(true);
    }
    #[cfg(not(feature = "jit"))]
    let _ = interpret_all;
    if interpreted {
        return function;
    }
    if let (Ok(function), Some(queue)) = (&function, engine.modules.borrow_mut().deferred.as_mut()) {
        queue.push_back(function.clone());
    }
    function
}

/// Gives `module` another source before anything requires it (a stand-in for a module a run
/// cannot have, e.g. one that needs the whole client); an error once it was required.
pub fn replace_source(engine: &Engine, module: InstanceId, source: &[u8]) -> Result<(), String> {
    if engine.modules.borrow().records.keys().any(|&(_, required)| required == module) {
        return Err(format!("{} was already required", engine.state.borrow().dom.full_name(&engine.reflection, module)));
    }
    engine.modules.borrow_mut().forget_bytecode(module);
    let mut state = engine.state.borrow_mut();
    let instance = state.dom.at_mut(module);
    instance.set_source(Some(Rc::from(source)));
    instance.set_source_path(None);
    Ok(())
}

/// A script's source text as a shared slice: its file under the mounted roots (read once), else its Source property.
/// When `compile_bytecode` is false, reading from disk skips eager standard-`require` bytecode compilation.
pub fn source_rc_opts(engine: &Engine, module: InstanceId, compile_bytecode: bool) -> Option<Rc<[u8]>> {
    let (source, pinned, path) = {
        let state = engine.state.borrow();
        let instance = state.dom.at(module);
        (instance.source().cloned(), instance.pinned_source().cloned(), instance.source_path().cloned())
    };
    if let Some(source) = source {
        return Some(source);
    }
    if let Some(pinned) = pinned {
        return Some(Rc::from(&*pinned));
    }
    if let Some(path) = path {
        note_read(engine, &path);
        if let Ok((bytes, bytecode)) = crate::timing::time("modules: read sources", || read_file_opts(engine, &path, compile_bytecode)) {
            let source: Rc<[u8]> = Rc::from(&*bytes);
            engine.state.borrow_mut().dom.at_mut(module).set_source(Some(source.clone()));
            if let Some(bytecode) = bytecode {
                engine.modules.borrow_mut().bytecode.insert(module, (source.clone(), bytecode));
            }
            return Some(source);
        }
    }
    match engine.state.borrow().dom.raw_get(module, K::Source as NameId) {
        Some(Value::String(source)) => Some(Rc::from(source.as_bytes())),
        _ => None,
    }
}

pub fn source_rc(engine: &Engine, module: InstanceId) -> Option<Rc<[u8]>> {
    source_rc_opts(engine, module, true)
}

/// A script's source text: its file under the mounted roots (read once), else its Source property.
pub fn source_of(engine: &Engine, module: InstanceId) -> Option<Vec<u8>> {
    {
        let state = engine.state.borrow();
        let instance = state.dom.at(module);
        if let Some(source) = instance.source() {
            return Some(source.to_vec());
        }
        if let Some(pinned) = instance.pinned_source() {
            return Some(pinned.to_vec());
        }
    }
    source_rc(engine, module).map(|source| source.to_vec())
}

/// Lists `path` among the files this run's modules came from ([`required_files`]).
pub fn note_required(engine: &Engine, path: &str) {
    engine.modules.borrow_mut().files.insert(path.to_string());
}

/// Parses a JSON module's file once per engine (and across workers when `share_compiled_modules` is active).
pub fn parse_json(engine: &Engine, path: &str) -> Result<std::sync::Arc<serde_json::Value>, String> {
    if let Some(cached) = engine.modules.borrow().json_cache.get(path).cloned() {
        return Ok(cached);
    }
    note_read(engine, path);
    if let Some(global_cache) = JSON_CACHE.get() {
        if is_batch_frozen() {
            if let Some(found) = global_cache.read().ok().and_then(|entries| entries.get(path).map(|(_, value)| value.clone())) {
                crate::files::note(std::path::Path::new(path));
                engine.modules.borrow_mut().json_cache.insert(path.to_string(), found.clone());
                return Ok(found);
            }
            let value = read_file_opts(engine, path, false)
                .map(|(bytes, _)| bytes)
                .map_err(|error| error.to_string())
                .and_then(|bytes| crate::project::data_files::read_json(&bytes))?;
            let cached = crate::alloc::long_lived(|| std::sync::Arc::new(value));
            let stamp = crate::files::Stamp { len: 0, modified: 0, directory: false };
            if let Ok(mut entries) = global_cache.write() {
                crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, cached.clone())));
            }
            engine.modules.borrow_mut().json_cache.insert(path.to_string(), cached.clone());
            return Ok(cached);
        }
        let file_path = std::path::Path::new(path);
        let stamp_opt = crate::files::stamp(file_path);
        if let Some(stamp) = stamp_opt {
            if let Some(found) = global_cache.read().ok().and_then(|entries| {
                entries.get(path).and_then(|(known, value)| (*known == stamp).then(|| value.clone()))
            }) {
                crate::files::note_stamp(file_path, stamp_opt);
                engine.modules.borrow_mut().json_cache.insert(path.to_string(), found.clone());
                return Ok(found);
            }
            let value = read_file_opts(engine, path, false)
                .map(|(bytes, _)| bytes)
                .map_err(|error| error.to_string())
                .and_then(|bytes| crate::project::data_files::read_json(&bytes))?;
            let cached = crate::alloc::long_lived(|| std::sync::Arc::new(value));
            if let Ok(mut entries) = global_cache.write() {
                crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, cached.clone())));
            }
            engine.modules.borrow_mut().json_cache.insert(path.to_string(), cached.clone());
            return Ok(cached);
        }
    }
    let value = read_file_opts(engine, path, false)
        .map(|(bytes, _)| bytes)
        .map_err(|error| error.to_string())
        .and_then(|bytes| crate::project::data_files::read_json(&bytes))?;
    let cached = std::sync::Arc::new(value);
    engine.modules.borrow_mut().json_cache.insert(path.to_string(), cached.clone());
    Ok(cached)
}

/// Parses a JSON module into compact binary JSON once per engine, process, and across processes
/// via the content-addressed mmap store.
pub(crate) fn parse_bjson(engine: &Engine, path: &str) -> Result<StoredBytes, String> {
    if let Some(cached) = engine.modules.borrow().bjson_cache.get(path).cloned() {
        return Ok(cached);
    }
    note_read(engine, path);
    let disk = store::for_engine(engine);
    if let Some(global_cache) = BJSON_CACHE.get() {
        if is_batch_frozen() || !keeping_between_runs() {
            if let Some(found) = global_cache.read().ok().and_then(|entries| entries.get(path).map(|(_, value)| value.clone())) {
                crate::files::note(std::path::Path::new(path));
                engine.modules.borrow_mut().bjson_cache.insert(path.to_string(), found.clone());
                return Ok(found);
            }
            let bytes = read_file_opts(engine, path, false).map(|(b, _)| b).map_err(|error| error.to_string())?;
            let bjson = parse_bjson_cached(disk.as_deref(), &bytes)?;
            let stamp = crate::files::Stamp { len: bytes.len() as u64, modified: 0, directory: false };
            if let Ok(mut entries) = global_cache.write() {
                if keeping_between_runs() {
                    crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, bjson.clone())));
                } else {
                    entries.insert(path.to_string(), (stamp, bjson.clone()));
                }
            }
            engine.modules.borrow_mut().bjson_cache.insert(path.to_string(), bjson.clone());
            return Ok(bjson);
        }
        let file_path = std::path::Path::new(path);
        let stamp_opt = crate::files::stamp(file_path);
        if let Some(stamp) = stamp_opt {
            if let Some(found) = global_cache.read().ok().and_then(|entries| {
                entries.get(path).and_then(|(known, value)| (*known == stamp).then(|| value.clone()))
            }) {
                crate::files::note_stamp(file_path, stamp_opt);
                engine.modules.borrow_mut().bjson_cache.insert(path.to_string(), found.clone());
                return Ok(found);
            }
            let bytes = read_file_opts(engine, path, false).map(|(b, _)| b).map_err(|error| error.to_string())?;
            let bjson = parse_bjson_cached(disk.as_deref(), &bytes)?;
            if let Ok(mut entries) = global_cache.write() {
                crate::alloc::long_lived(|| entries.insert(path.to_string(), (stamp, bjson.clone())));
            }
            engine.modules.borrow_mut().bjson_cache.insert(path.to_string(), bjson.clone());
            return Ok(bjson);
        }
    }
    let bytes = read_file_opts(engine, path, false).map(|(b, _)| b).map_err(|error| error.to_string())?;
    let bjson = parse_bjson_cached(disk.as_deref(), &bytes)?;
    engine.modules.borrow_mut().bjson_cache.insert(path.to_string(), bjson.clone());
    Ok(bjson)
}

/// Converts a parsed JSON value into a fresh Luau value (no shared mutable tables between calls).
pub fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> Result<LuaValue, String> {
    store::serde_json_to_lua(lua, value)
}

/// Returns `(function, cleanup)` for `debug.loadmodule` on a JSON module, reusing one loaded
/// template closure per `(module, path, full_name, payload)` and cloning it into a fresh per-load
/// environment via `lua_clonefunction` + `lua_setfenv` instead of reloading bytecode and closures
/// on every call.
pub(crate) fn load_json_loadmodule(
    lua: &Lua,
    engine: &Rc<Engine>,
    module: InstanceId,
    path: &Rc<str>,
) -> LuaResult<Result<(LuaFunction, LuaFunction), String>> {
    let name = engine.state.borrow().dom.full_name(&engine.reflection, module);
    let bjson = match parse_bjson(engine, path) {
        Ok(value) => value,
        Err(message) if message == JSON_RUNS_SOURCE => {
            let source = match json_source_instead(engine, module, path) {
                Ok(source) => source,
                Err(message) => return Ok(Err(invalid_json_module(&name, &message))),
            };
            return Ok(match compile_for_loadmodule(lua, engine, module, &source) {
                Ok(function) => Ok((function, noop_cleanup(lua, engine)?)),
                Err(error) => Err(crate::scheduler::error_message(&error)),
            });
        }
        Err(message) => return Ok(Err(invalid_json_module(&name, &message))),
    };
    let bjson_ptr = bjson.ptr_id();
    let cached_template = engine
        .modules
        .borrow()
        .json_loadmodule_templates
        .get(&module)
        .and_then(|(cached_path, cached_name, cached_ptr, template)| {
            (cached_path == path && cached_name == &name && *cached_ptr == bjson_ptr).then(|| template.clone())
        });
    let env = environment(lua, engine, module)?;
    let function = match cached_template {
        Some(template) => clone_with_env(lua, &template, &env)?,
        None => {
            let payload = bjson.clone();
            let decode = lua.create_function(move |lua, ()| store::bjson_to_lua(lua, &payload).map_err(LuaError::runtime))?;
            let template_env = engine.lua.borrow().shared.clone().expect("shared globals");
            let template = wrap_json_loadmodule(lua, engine, &name, decode, template_env)?;
            engine
                .modules
                .borrow_mut()
                .json_loadmodule_templates
                .insert(module, (path.clone(), name, bjson_ptr, template.clone()));
            clone_with_env(lua, &template, &env)?
        }
    };
    let cleanup = noop_cleanup(lua, engine)?;
    Ok(Ok((function, cleanup)))
}

/// Loads the tiny precompiled JSON module wrapper with the module's `={name}` chunk name (`debug.info(fn, "s")`) and
/// per-load environment. Under `--native` it skips JIT codegen (avoiding thousands of redundant machine-code compiles
/// for data-only JSON modules); otherwise it loads as every module chunk does ([`load`]), so a VM that compiles or
/// defers native code still runs it as before.
pub fn wrap_json_loadmodule(lua: &Lua, engine: &Rc<Engine>, name: &str, decode: LuaFunction, env: LuaTable) -> LuaResult<LuaFunction> {
    if !engine.options.native {
        let chunk = lua.load(json_wrapper_bytecode()).set_mode(mlua::chunk::ChunkMode::Binary).set_name(format!("={name}")).set_environment(env);
        return load(lua, engine, chunk)?.call(decode);
    }
    #[cfg(feature = "jit")]
    let keep_jit_off = {
        let m = engine.modules.borrow();
        m.interpret_all || m.tiered_jit
    };
    #[cfg(feature = "jit")]
    lua.enable_jit(false);
    let factory = crate::timing::time("modules: compile + load", || {
        lua.load(json_wrapper_bytecode())
            .set_mode(mlua::chunk::ChunkMode::Binary)
            .set_name(format!("={name}"))
            .set_environment(env)
            .into_function()
    });
    #[cfg(feature = "jit")]
    if !keep_jit_off {
        lua.enable_jit(true);
    }
    factory?.call(decode)
}

/// Single stateless no-op cleanup function shared across `debug.loadmodule` calls in this VM.
pub fn noop_cleanup(lua: &Lua, engine: &Engine) -> LuaResult<LuaFunction> {
    if let Some(cleanup) = engine.modules.borrow().noop_cleanup.clone() {
        return Ok(cleanup);
    }
    let cleanup = lua.create_function(|_, _: LuaMultiValue| Ok(()))?;
    engine.modules.borrow_mut().noop_cleanup = Some(cleanup.clone());
    Ok(cleanup)
}

/// Compiled bytecode for the tiny wrapper chunk `debug.loadmodule` returns for JSON modules (compiled once per process).
pub fn json_wrapper_bytecode() -> &'static [u8] {
    static WRAPPER: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    WRAPPER.get_or_init(|| {
        crate::alloc::long_lived(|| {
            compiler()
                .compile("local Decode = ...\nreturn function()\n\treturn Decode()\nend")
                .expect("json wrapper bytecode")
        })
    })
}

/// The error a require of a JSON module that does not read raises: the reason `rojo build` refuses it
pub(crate) fn invalid_json_module(name: &str, reason: &str) -> String {
    format!("{name}: invalid JSON module (File contains malformed JSON: {reason}; Rojo refuses the build)")
}

/// The source Rojo writes for a JSON module that runs it ([`JSON_RUNS_SOURCE`]), which the module keeps from now on: its
/// Source, and no longer a JSON module
fn json_source_instead(engine: &Engine, module: InstanceId, path: &str) -> Result<Rc<[u8]>, String> {
    let bytes = read_file_opts(engine, path, false).map(|(bytes, _)| bytes).map_err(|error| error.to_string())?;
    let source: Rc<[u8]> = Rc::from(crate::project::data_files::json_module_source(&bytes)?.into_bytes());
    let mut state = engine.state.borrow_mut();
    let instance = state.dom.at_mut(module);
    instance.set_source(Some(source.clone()));
    instance.set_flag(flags::JSON_MODULE, false);
    Ok(source)
}

/// A JSON module's value (Rojo `.json`): the file decoded, a new value each time.
pub fn decode_json(lua: &Lua, engine: &Engine, path: &str) -> Result<LuaValue, String> {
    let bjson = parse_bjson(engine, path)?;
    store::bjson_to_lua(lua, &bjson)
}

fn error_result(lua: &Lua, message: String, level: i64) -> LuaResult<LuaMultiValue> {
    Ok(LuaMultiValue::from_iter([
        LuaValue::Integer(1),
        LuaValue::String(lua.create_string(message)?),
        LuaValue::Integer(level as _),
    ]))
}

/// Resolves a require-by-string path from the requiring script (`caller`: the calling Luau function's environment's `script`,
/// None when that is no Instance), as a client does (measured, a 0.747 playing client and a server:
/// tools/parity/probes/script_env.luau). `./` starts at the script's parent and `../` there too, its own `..` then going up once
/// more; `@self` starts at the script and `@game` at the DataModel (an alias is named in any case; any other raises, in lower
/// case). Each further component is a child by its exact name (no file extension dropped), `..` a parent, and `.` or an empty
/// component nothing; a backslash separates as `/` does. A `..` that finds no parent raises naming the component before it, or
/// the requiring context when there is none. Without a script the requiring context is a place of its own, with no parent and no
/// children: `@self` ends there (None), as no module. What the path ends at must be a ModuleScript (require_begin).
fn resolve_path(engine: &Rc<Engine>, path: &[u8], caller: Option<InstanceId>) -> Result<Option<InstanceId>, String> {
    let text = String::from_utf8_lossy(path).into_owned();
    let fail = |reason: String| format!("error requiring \"{text}\": {reason}");
    let parent_of = |id: Option<InstanceId>| id.and_then(|id| engine.state.borrow().dom.at(id).parent);
    const NO_CONTEXT_PARENT: &str = "could not get parent of requiring context";
    let path = text.replace('\\', "/");
    let first = path.split('/').next().unwrap_or("");
    // (where the path starts, and its components from there: a `./` or `../` path's own first component included)
    let (mut current, components) = if first == "." || first == ".." {
        if !path.contains('/') {
            return Err(fail(PREFIX.into()));
        }
        let Some(start) = parent_of(caller) else { return Err(fail(NO_CONTEXT_PARENT.into())) };
        (Some(start), path.as_str())
    } else if let Some(alias) = first.strip_prefix('@') {
        let lower = alias.to_lowercase();
        let start = match lower.as_str() {
            "self" => caller,
            "game" => Some(engine.world().game),
            _ => return Err(fail(format!("@{lower} is not a valid alias"))),
        };
        (start, &path[first.len()..])
    } else {
        return Err(fail(PREFIX.into()));
    };
    let mut previous: Option<&str> = None;
    for component in components.split('/') {
        if component == "." || component.is_empty() {
            continue;
        }
        if component == ".." {
            current = Some(parent_of(current).ok_or_else(|| match previous {
                Some(previous) => fail(format!("could not get parent of component \"{previous}\"")),
                None => fail(NO_CONTEXT_PARENT.into()),
            })?);
        } else {
            let found = current.and_then(|node| {
                materialize(engine, node);
                engine.state.borrow().dom.find_child(node, component.as_bytes())
            });
            current = Some(found.ok_or_else(|| fail(format!("could not resolve child component \"{component}\"")))?);
        }
        previous = Some(component);
    }
    Ok(current)
}

/// What a path that starts with no prefix raises after its `error requiring "<path>": `
const PREFIX: &str = "require path must start with a valid prefix: ./, ../, or @";

fn thread_key(thread: &LuaThread) -> usize {
    thread.to_pointer() as usize
}

/// RequireBegin(Target, Caller): see prelude.luau Require.
pub fn require_begin(lua: &Lua, engine: &Rc<Engine>, (target, caller, vm): (LuaValue, LuaValue, Vm)) -> LuaResult<LuaMultiValue> {
    let caller_id = match &caller {
        LuaValue::UserData(ud) => instance_of_ud(ud),
        _ => None,
    };
    let module = match &target {
        LuaValue::String(path) => {
            // (the requiring script is the calling Luau function's environment's `script`: lua/builtins.rs require_function; none
            // when no Luau function called, false)
            if matches!(caller, LuaValue::Boolean(false)) {
                return error_result(lua, NO_LUAU_CALLER.into(), 0);
            }
            match resolve_path(engine, &path.as_bytes(), caller_id) {
                Ok(Some(module)) => module,
                Ok(None) => return error_result(lua, format!("Unable to require module from given path '{}'", path.to_string_lossy()), 0),
                Err(message) => return error_result(lua, message, 0),
            }
        }
        LuaValue::Number(_) | LuaValue::Integer(_) => {
            let text = lua.coerce_string(target.clone())?.map(|s| s.to_string_lossy()).unwrap_or_default();
            // (a playing client's text, measured, Studio 0.744: tools/parity/probes/errors.luau; a server downloads the asset,
            // which ree cannot offline)
            if !engine.options.server() {
                return error_result(lua, format!("require(assetId) cannot be called from a client.  assetId = {text}"), 0);
            }
            return error_result(lua, format!("require(assetId) is not supported offline (asset {text})"), 0);
        }
        LuaValue::UserData(ud) if instance_of_ud(ud).is_some() => instance_of_ud(ud).unwrap(),
        _ => return error_result(lua, "Attempted to call require with invalid argument(s).".into(), 0),
    };
    let module_script = engine.class_id("ModuleScript");
    if Some(engine.state.borrow().dom.at(module).class) != module_script {
        if let LuaValue::String(path) = &target {
            return error_result(lua, format!("Unable to require module from given path '{}'", path.to_string_lossy()), 0);
        }
        return error_result(lua, "Attempted to call require with invalid argument(s).".into(), 0);
    }

    let current = lua.current_thread();
    {
        let mut refs = engine.modules.borrow_mut();
        let loading_now = refs.loading_now.contains(&(vm, module));
        if let Some(record) = refs.records.get_mut(&(vm, module)) {
            match record.status {
                Status::Done => {
                    crate::profiler::caches::hit(crate::profiler::caches::Cache::Requires);
                    // (a module required again: cost.rs REQUIRE_KEPT)
                    engine.state.borrow().sched.charge(crate::cost::REQUIRE_KEPT);
                    return Ok(LuaMultiValue::from_iter([LuaValue::Integer(0), record.result.clone().unwrap_or(LuaValue::Nil)]));
                }
                Status::Error => {
                    return error_result(lua, record.failure.into(), 0);
                }
                Status::Loading => {
                    // (recursive: the module's chunk is running in the chain of loads this require is part of, the chunk itself or a
                    // thread it runs without a yield between, however deep (a cycle of modules raises in its last module). A thread
                    // the chunk's thread resumes after it yielded, or any other thread, waits; so a cycle with a yield in it waits
                    // for ever, as on a client: measured, a 0.747 client and server, tools/parity/probes/engine_threads.luau)
                    if loading_now || record.owner.as_ref().map(thread_key) == Some(thread_key(&current)) {
                        return error_result(lua, RECURSIVE.into(), 0);
                    }
                    // (a thread that cannot yield does not wait for it: lua/mod.rs NOT_YIELDABLE; measured in a 0.747
                    // playing client and server, tools/parity/probes/yield_refused_require.luau)
                    if !super::yieldable(&current) {
                        return error_result(lua, super::NOT_YIELDABLE.into(), 0);
                    }
                    record.waiting.push(current);
                    return Ok(LuaMultiValue::from_iter([LuaValue::Integer(2)]));
                }
            }
        }
    }
    crate::profiler::caches::miss(crate::profiler::caches::Cache::Requires);
    // JSON modules (Rojo `.json`) evaluate to the decoded value
    let (json, path, studio_only) = {
        let state = engine.state.borrow();
        let instance = state.dom.at(module);
        (instance.has(flags::JSON_MODULE), instance.source_path().cloned(), instance.has(flags::STUDIO_ONLY))
    };
    if let Some(path) = &path {
        engine.modules.borrow_mut().files.insert(path.to_string());
    }
    // (a JSON module that runs the source Rojo writes for it takes that source here, and loads below as a script would)
    let runs_source = json && path.as_ref().is_some_and(|path| matches!(parse_bjson(engine, path), Err(message) if message == JSON_RUNS_SOURCE));
    if let (true, false, Some(path)) = (json, runs_source, &path) {
        let decoded = decode_json(lua, engine, path);
        let mut refs = engine.modules.borrow_mut();
        return match decoded {
            Ok(value) => {
                refs.records.insert((vm, module), Record { status: Status::Done, result: Some(value.clone()), ..Record::loading() });
                Ok(LuaMultiValue::from_iter([LuaValue::Integer(0), value]))
            }
            Err(message) => {
                refs.records.insert((vm, module), Record { status: Status::Error, ..Record::loading() });
                drop(refs);
                let name = engine.state.borrow().dom.full_name(&engine.reflection, module);
                error_result(lua, invalid_json_module(&name, &message), 0)
            }
        };
    }
    if let (true, Some(path)) = (runs_source, &path) {
        if let Err(message) = json_source_instead(engine, module, path) {
            let name = engine.state.borrow().dom.full_name(&engine.reflection, module);
            engine.modules.borrow_mut().records.insert((vm, module), Record { status: Status::Error, ..Record::loading() });
            return error_result(lua, invalid_json_module(&name, &message), 0);
        }
    }

    // File-backed modules with no in-memory override borrow the shared `Arc<[u8]>` source and
    // precompiled bytecode directly without copying the source bytes into a per-instance `Rc<[u8]>`,
    // while storing the `Arc<[u8]>` on the instance so `.Source`, `Clone()`, and Actor VMs all keep
    // seeing the first-read text for the rest of the session.
    let (in_memory_source, pinned) = {
        let state = engine.state.borrow();
        let instance = state.dom.at(module);
        (instance.source().cloned(), instance.pinned_source().cloned())
    };
    let loaded = match (&in_memory_source, &pinned, &path) {
        (Some(source), _, _) => Some((None, Some(source.clone()), None)),
        (None, Some(pinned), _) => Some((Some(pinned.clone()), None, None)),
        (None, None, Some(path)) => {
            note_read(engine, path);
            match crate::timing::time("modules: read sources", || read_file_opts(engine, path, true)) {
                Ok((bytes, bytecode)) => {
                    engine.state.borrow_mut().dom.at_mut(module).set_pinned_source(Some(bytes.clone()));
                    Some((Some(bytes), None, bytecode))
                }
                Err(_) => match engine.state.borrow().dom.raw_get(module, K::Source as NameId) {
                    Some(Value::String(source)) => Some((None, Some(Rc::from(source.as_bytes())), None)),
                    _ => None,
                },
            }
        }
        (None, None, None) => match engine.state.borrow().dom.raw_get(module, K::Source as NameId) {
            Some(Value::String(source)) => Some((None, Some(Rc::from(source.as_bytes())), None)),
            _ => None,
        },
    };
    let Some((arc_source, rc_source, precompiled)) = loaded else {
        let name = engine.state.borrow().dom.full_name(&engine.reflection, module);
        let message = if studio_only {
            format!("Module {name} exists only in Studio (its source is not in the project)")
        } else {
            format!("Module {name} has no source")
        };
        return error_result(lua, message, 0);
    };
    let source_bytes: &[u8] = match (&arc_source, &rc_source) {
        (Some(bytes), _) => bytes,
        (None, Some(bytes)) => bytes,
        (None, None) => unreachable!(),
    };

    engine.modules.borrow_mut().records.insert((vm, module), Record::loading());
    // (a module's first require loads its chunk: cost.rs required, by its source's bytes)
    let loading = crate::cost::required(source_bytes.len(), engine.options.server());
    match compile_in_with(lua, engine, module, source_bytes, precompiled, vm, true) {
        Ok(chunk) => {
            engine.state.borrow().sched.charge(loading);
            Ok(LuaMultiValue::from_iter([LuaValue::Integer(3), LuaValue::Function(chunk), instance_value(lua, engine, module)?]))
        }
        Err(error) => {
            // A source that does not compile fails the module as an error in its chunk does: the compiler's message (named by the
            // module itself) is logged once as an error, with no stack and no ScriptContext.Error, and this require and every later
            // one raise LOAD_FAILED, a fixed Source too (measured, a 0.747 playing client: tools/parity/probes/script_env.luau)
            let message = super::super::scheduler::error_message(&error);
            engine.output_error_with_script(message, None, None, false);
            finish(lua, engine, vm, module, Err(LOAD_FAILED));
            error_result(lua, LOAD_FAILED.into(), 0)
        }
    }
}

/// Records a module's outcome and wakes the threads waiting for it at once, in the order they began to wait (the first requirer
/// first), before deferred work its chunk queued (measured, a 0.747 client and server: tools/parity/probes/engine_threads.luau)
fn finish(lua: &Lua, engine: &Rc<Engine>, vm: Vm, module: InstanceId, result: Result<LuaValue, &'static str>) {
    let waiting = {
        let mut refs = engine.modules.borrow_mut();
        let Some(record) = refs.records.get_mut(&(vm, module)) else { return };
        match result {
            Ok(value) => {
                record.status = Status::Done;
                record.result = Some(value);
            }
            Err(failure) => {
                record.status = Status::Error;
                record.failure = failure;
            }
        }
        record.owner = None;
        std::mem::take(&mut record.waiting)
    };
    for thread in waiting {
        crate::scheduler::resume(lua, engine, &thread, LuaMultiValue::new());
    }
}

/// RequireRun(Chunk, Module): runs the module's chunk RequireBegin answered with (status 3) on a thread of its own, as a client
/// does (measured, a 0.747 client and server: tools/parity/probes/engine_threads.luau): the chunk is that thread's bottom, so
/// `debug.info(2)` in it is nil and a traceback in it lists the module alone; the requiring thread is "normal" while it runs; the
/// thread starts with the module's memory category (its script's Name), and an error it raises is logged as that thread's uncaught
/// error (its own stack, ScriptContext.Error naming the module), before or after a yield alike. Answers as RequireBegin does: 0 and
/// the module's value, 1 with the message every require of it raises (level 0), or 2 when the chunk yielded: the requiring thread
/// waits for it (resumed when it ends, then asks again), or raises `thread is not yieldable` where it cannot yield while the chunk
/// goes on (measured: tools/parity/probes/yield_refused_require.luau). The chunk's thread is the module's owner, and the module is
/// in the chain of loads running now ([`ModuleRecords::loading_now`]) until it returns or first yields.
pub fn require_run(lua: &Lua, engine: &Rc<Engine>, (chunk, module, vm): (LuaFunction, LuaValue, Vm)) -> LuaResult<LuaMultiValue> {
    let LuaValue::UserData(ud) = module else { return Err(LuaError::runtime("RequireRun: module expected")) };
    let module = instance_of_ud(&ud).ok_or_else(|| LuaError::runtime("RequireRun: module expected"))?;
    let globals = chunk.environment();
    let thread = lua.create_thread(chunk)?;
    // (the chunk's thread reads the module's environment as its globals and belongs to no script's run: a 0.747 client's module
    // chunk reads getfenv(0) as its own, and what it starts goes on when the requiring script stops: scripts.rs)
    crate::scripts::set_thread(lua, &thread, globals.as_ref(), 0);
    {
        let mut refs = engine.modules.borrow_mut();
        if let Some(record) = refs.records.get_mut(&(vm, module)) {
            record.owner = Some(thread.clone());
        }
        refs.loading_now.push((vm, module));
    }
    let ran = crate::scheduler::resume_own(lua, engine, &thread, LuaMultiValue::new(), crate::scheduler::Report::Uncaught);
    {
        let mut refs = engine.modules.borrow_mut();
        if let Some(index) = refs.loading_now.iter().rposition(|&entry| entry == (vm, module)) {
            refs.loading_now.remove(index);
        }
    }
    if let crate::scheduler::Ran::Yielded = ran {
        engine.state.borrow_mut().sched.await_end(&thread, crate::scheduler::ThreadEnd::Module { vm, module });
        let current = lua.current_thread();
        if !super::yieldable(&current) {
            return error_result(lua, super::NOT_YIELDABLE.into(), 0);
        }
        if let Some(record) = engine.modules.borrow_mut().records.get_mut(&(vm, module)) {
            record.waiting.push(current);
        }
        return Ok(LuaMultiValue::from_iter([LuaValue::Integer(2)]));
    }
    match chunk_outcome(lua, engine, vm, module, ran) {
        Ok(value) => Ok(LuaMultiValue::from_iter([LuaValue::Integer(0), value])),
        Err(failure) => error_result(lua, failure.into(), 0),
    }
}

/// A module's chunk ended after it yielded (scheduler.rs ThreadEnd::Module): its outcome finishes the module
pub(crate) fn chunk_ended(lua: &Lua, engine: &Rc<Engine>, vm: Vm, module: InstanceId, ran: crate::scheduler::Ran) {
    let _ = chunk_outcome(lua, engine, vm, module, ran);
}

/// What a module's chunk ended with makes the module's value, or its failure (one value returned, or [`NOT_ONE_VALUE`]; an error
/// it raised was logged as its thread's: [`LOAD_FAILED`]), and wakes the threads waiting for it
fn chunk_outcome(lua: &Lua, engine: &Rc<Engine>, vm: Vm, module: InstanceId, ran: crate::scheduler::Ran) -> Result<LuaValue, &'static str> {
    let outcome = match ran {
        crate::scheduler::Ran::Returned(mut values) if values.len() == 1 => Ok(values.pop_front().unwrap_or(LuaValue::Nil)),
        crate::scheduler::Ran::Returned(_) | crate::scheduler::Ran::Yielded => Err(NOT_ONE_VALUE),
        crate::scheduler::Ran::Raised(_) => {
            // (the error a client reports for the module: cost.rs REQUIRE_FAILED)
            engine.state.borrow().sched.charge(crate::cost::REQUIRE_FAILED[usize::from(engine.options.server())]);
            Err(LOAD_FAILED)
        }
    };
    finish(lua, engine, vm, module, outcome.clone());
    outcome
}

/// Modules.Require from the engine (no calling script): runs on the current thread.
pub fn require(lua: &Lua, engine: &Rc<Engine>, module: InstanceId) -> LuaResult<LuaValue> {
    let prelude = engine.prelude();
    let require: LuaFunction = prelude.exports.get("Require")?;
    require.call((instance_value(lua, engine, module)?, LuaValue::Nil, LuaValue::Nil))
}

/// Each module's result, for the modules that returned (diagnostics: `lua::holders`).
impl ModuleRecords {
    pub fn results(&self) -> Vec<(InstanceId, LuaValue)> {
        self.records.iter().filter(|((vm, _), _)| vm.is_none()).filter_map(|(&(_, module), record)| Some((module, record.result.clone()?))).collect()
    }
}

/// Modules.Reset
pub fn reset(engine: &Engine, module: InstanceId) {
    engine.modules.borrow_mut().records.retain(|&(_, required), _| required != module);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stamp the mount saved when it listed a module is not the file's: a module edited after the listing is read
    /// again (source and bytecode), and one deleted after it is not found.
    #[test]
    fn a_compiled_module_follows_the_file_not_its_listing() {
        reuse_compiled_modules();
        let directory = std::env::temp_dir().join(format!("ree-compiled-source-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("Module.luau");
        let path = file.to_string_lossy().into_owned();
        std::fs::write(&file, "return 'before 1'").unwrap();
        let listed_stamp = crate::files::stamp(&file).unwrap();
        let engine = Engine::new(crate::Options::default()).unwrap();
        engine.state.borrow_mut().project.stamps.insert(path.clone(), listed_stamp);
        let original = read_file(&engine, &path).unwrap();
        // (the new source is two bytes longer, so its stamp differs whatever the clock's resolution)
        std::fs::write(&file, "return 'before two'").unwrap();
        assert_ne!(crate::files::stamp(&file), Some(listed_stamp), "the edit changes the file's stamp");
        let edited = read_file(&engine, &path).unwrap();
        std::fs::remove_file(&file).unwrap();
        let deleted = read_file(&engine, &path);
        std::fs::remove_dir(&directory).unwrap();
        assert_eq!(&*original.0, b"return 'before 1'");
        assert_eq!(&*edited.0, b"return 'before two'");
        assert_eq!(Lua::new().load(&*edited.1.unwrap()).eval::<String>().unwrap(), "before two");
        assert_eq!(deleted.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }

    /// If a background worker panics while owning an in-flight engine Luau slot, waiting callers
    /// wake up, the dead slot is removed, and the waiter compiles the chunk itself.
    #[test]
    fn a_panicking_engine_compile_wakes_waiters_and_falls_back() {
        static TEST_SOURCE: &str = "return 42";
        let key = TEST_SOURCE.as_ptr() as usize;
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let owner = std::thread::spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = ensure_engine_bytecode_with(TEST_SOURCE, false, |_| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    panic!("simulated compiler panic");
                });
            }));
        });
        entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("owner entered compile");
        let (waiter_tx, waiter_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let res = ensure_engine_bytecode_with(TEST_SOURCE, true, |src| {
                engine_compiler().compile(src).map_err(|e| e.to_string())
            });
            let _ = waiter_tx.send(res);
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let waiters_joined = ENGINE_INFLIGHT
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|(at, _)| *at == key)
                .is_some_and(|(_, slot)| std::sync::Arc::strong_count(slot) >= 3);
            if waiters_joined || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::yield_now();
        }
        release_tx.send(()).unwrap();
        let bytecode = waiter_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("waiter hung after owner panicked")
            .expect("foreground fallback compile");
        owner.join().unwrap();
        waiter.join().unwrap();
        assert_eq!(Lua::new().load(&*bytecode).eval::<i64>().unwrap(), 42);
        let still_inflight = ENGINE_INFLIGHT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|(at, _)| *at == key);
        assert!(!still_inflight, "inflight entry leaked after panic");
        if let Ok(mut kept) = ENGINE_BYTECODE.lock() {
            kept.retain(|(at, _)| *at != key);
        }
    }
}
