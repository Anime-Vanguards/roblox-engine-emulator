//! Persistent cross-process content-addressed module store (`<state_dir>/module-store/v<N>-<abi>/packs/`).
//!
//! Stores compiled Luau bytecode (for both `require` and mutable-sandbox `debug.loadmodule`) and
//! compact binary JSON payloads in immutable, read-only memory-mapped pack files (`.mpack`).
//! Windows maps identical read-only file mappings onto the same physical pages across processes,
//! eliminating per-process recompilation and private-memory duplication without locks or queues.
//!
//! Safety invariants:
//! - Every entry is keyed by `(kind, sha256(content), content_len)` and scoped to a versioned
//!   directory + header containing the format version and live compiler ABI digest.
//! - Writers serialize to a unique temporary file beside the target and atomically rename into a
//!   content-addressed `.mpack` name; a crashed writer leaves an ignored `.tmp` file, and concurrent
//!   writers of the same pack converge on the identical immutable file without blocking.
//! - Readers validate magic, format version, ABI digest, bounds, and index ordering on open,
//!   silently ignoring incompatible or corrupted files.

use std::{
    ffi::{OsString, c_char, c_int},
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use mlua::{ffi, prelude::*};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::project::plan::Mapping;

pub(crate) const KIND_STD_BYTECODE: u8 = 1;
pub(crate) const KIND_LOADMODULE_BYTECODE: u8 = 2;
pub(crate) const KIND_BINARY_JSON: u8 = 3;

const PACK_MAGIC: &[u8; 8] = b"REEMPK01";
const FORMAT_VERSION: u32 = 1;
const HEADER_LEN: usize = 48;
const RECORD_LEN: usize = 56;
const COMPACT_THRESHOLD: usize = 8;
const STALE_TMP_AGE: Duration = Duration::from_secs(300);

const BJSON_MAGIC: &[u8; 4] = b"RBJ1";
const TAG_NULL: u8 = 0;
const TAG_FALSE: u8 = 1;
const TAG_TRUE: u8 = 2;
const TAG_NUMBER: u8 = 3;
const TAG_STRING: u8 = 4;
const TAG_ARRAY: u8 = 5;
const TAG_OBJECT: u8 = 6;
const MAX_JSON_DEPTH: usize = 256;

/// Immutable content-addressed key for an artifact in a pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct EntryKey {
    pub kind: u8,
    pub hash: [u8; 32],
    pub source_len: u64,
}

/// Backing storage for bytes read out of the store (either a read-only OS file mapping or heap).
pub(crate) enum PackBacking {
    Mapped(Arc<Mapping>),
    Owned(Arc<[u8]>),
}

impl PackBacking {
    #[inline]
    fn bytes(&self) -> &[u8] {
        match self {
            PackBacking::Mapped(mapping) => mapping.bytes(),
            PackBacking::Owned(bytes) => bytes,
        }
    }
}

/// A cheap-to-clone immutable byte slice pointing either directly into a shared read-only OS file
/// mapping (zero private RAM, zero copy) or an in-memory `Arc<[u8]>`.
#[derive(Clone)]
pub(crate) struct StoredBytes {
    backing: Arc<PackBacking>,
    start: usize,
    end: usize,
}

impl StoredBytes {
    pub(crate) fn owned(bytes: Arc<[u8]>) -> Self {
        let end = bytes.len();
        Self { backing: Arc::new(PackBacking::Owned(bytes)), start: 0, end }
    }

    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self::owned(Arc::from(bytes.into_boxed_slice()))
    }

    #[inline]
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.backing.bytes()[self.start..self.end]
    }

    #[inline]
    pub(crate) fn ptr_id(&self) -> usize {
        self.as_slice().as_ptr() as usize
    }
}

impl Deref for StoredBytes {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::fmt::Debug for StoredBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredBytes").field("len", &self.as_slice().len()).finish()
    }
}

/// One immutable `.mpack` file validated and mapped read-only.
struct Pack {
    path: PathBuf,
    backing: Arc<PackBacking>,
    entry_count: usize,
}

impl Pack {
    fn open(path: &Path, expected_abi: &[u8; 32]) -> Option<Arc<Self>> {
        let backing = match Mapping::open(path) {
            Some(mapping) => Arc::new(PackBacking::Mapped(mapping)),
            None => {
                let bytes = std::fs::read(path).ok()?;
                Arc::new(PackBacking::Owned(Arc::from(bytes.into_boxed_slice())))
            }
        };
        let bytes = backing.bytes();
        if bytes.len() < HEADER_LEN {
            return None;
        }
        if &bytes[0..8] != PACK_MAGIC {
            return None;
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
        if version != FORMAT_VERSION {
            return None;
        }
        let entry_count = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
        if &bytes[16..48] != expected_abi {
            return None;
        }
        let table_end = HEADER_LEN.checked_add(entry_count.checked_mul(RECORD_LEN)?)?;
        if table_end > bytes.len() {
            return None;
        }
        // Validate bounds and strict sorted order once on open so binary search & slicing are infallible.
        let mut prev: Option<EntryKey> = None;
        for index in 0..entry_count {
            let (key, offset, len) = read_record(bytes, index);
            if let Some(prev_key) = prev {
                if key <= prev_key {
                    return None;
                }
            }
            let end = offset.checked_add(len)?;
            if offset < table_end || end > bytes.len() {
                return None;
            }
            prev = Some(key);
        }
        Some(Arc::new(Pack { path: path.to_path_buf(), backing, entry_count }))
    }

    #[inline]
    fn get(&self, target: &EntryKey) -> Option<StoredBytes> {
        let bytes = self.backing.bytes();
        let mut lo = 0usize;
        let mut hi = self.entry_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let (key, offset, len) = read_record(bytes, mid);
            match key.cmp(target) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => {
                    return Some(StoredBytes { backing: self.backing.clone(), start: offset, end: offset + len });
                }
            }
        }
        None
    }

    fn iter_entries(&self) -> impl Iterator<Item = (EntryKey, &[u8])> {
        let bytes = self.backing.bytes();
        (0..self.entry_count).map(move |index| {
            let (key, offset, len) = read_record(bytes, index);
            (key, &bytes[offset..offset + len])
        })
    }
}

#[inline]
fn read_record(bytes: &[u8], index: usize) -> (EntryKey, usize, usize) {
    let base = HEADER_LEN + index * RECORD_LEN;
    let kind = bytes[base];
    let payload_len = u32::from_le_bytes([bytes[base + 4], bytes[base + 5], bytes[base + 6], bytes[base + 7]]) as usize;
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes[base + 8..base + 40]);
    let source_len = u64::from_le_bytes([
        bytes[base + 40], bytes[base + 41], bytes[base + 42], bytes[base + 43],
        bytes[base + 44], bytes[base + 45], bytes[base + 46], bytes[base + 47],
    ]);
    let payload_offset = u64::from_le_bytes([
        bytes[base + 48], bytes[base + 49], bytes[base + 50], bytes[base + 51],
        bytes[base + 52], bytes[base + 53], bytes[base + 54], bytes[base + 55],
    ]) as usize;
    (EntryKey { kind, hash, source_len }, payload_offset, payload_len)
}

fn serialize_pack(entries: &[(EntryKey, &[u8])], abi: &[u8; 32]) -> Option<Vec<u8>> {
    let count = u32::try_from(entries.len()).ok()?;
    let table_end = HEADER_LEN + entries.len() * RECORD_LEN;
    let payload_total: usize = entries.iter().map(|(_, p)| p.len()).sum();
    let mut buf = Vec::with_capacity(table_end + payload_total);
    buf.extend_from_slice(PACK_MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(abi);
    let mut offset = table_end as u64;
    for (key, payload) in entries {
        let len = u32::try_from(payload.len()).ok()?;
        buf.push(key.kind);
        buf.extend_from_slice(&[0u8; 3]);
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&key.hash);
        buf.extend_from_slice(&key.source_len.to_le_bytes());
        buf.extend_from_slice(&offset.to_le_bytes());
        offset += payload.len() as u64;
    }
    for (_, payload) in entries {
        buf.extend_from_slice(payload);
    }
    Some(buf)
}

/// Cross-process content-addressed store backed by immutable `.mpack` files.
pub(crate) struct Store {
    packs_dir: PathBuf,
    abi: [u8; 32],
    packs: RwLock<Vec<Arc<Pack>>>,
    loaded: Mutex<FxHashSet<OsString>>,
    pending: Mutex<FxHashMap<EntryKey, Arc<[u8]>>>,
    has_pending: AtomicBool,
    flush_lock: Mutex<()>,
}

impl Store {
    pub(crate) fn open_at(versioned_dir: PathBuf, abi: [u8; 32]) -> Arc<Self> {
        let packs_dir = versioned_dir.join("packs");
        let store = Arc::new(Self {
            packs_dir,
            abi,
            packs: RwLock::new(Vec::new()),
            loaded: Mutex::new(FxHashSet::default()),
            pending: Mutex::new(FxHashMap::default()),
            has_pending: AtomicBool::new(false),
            flush_lock: Mutex::new(()),
        });
        store.refresh_from_disk();
        store
    }

    fn refresh_from_disk(&self) {
        let Ok(read_dir) = std::fs::read_dir(&self.packs_dir) else { return };
        let mut candidates: Vec<(OsString, PathBuf, u64)> = Vec::new();
        {
            let loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
            for entry in read_dir.flatten() {
                let name = entry.file_name();
                let Some(name_str) = name.to_str() else { continue };
                if !name_str.ends_with(".mpack") || loaded.contains(&name) {
                    continue;
                }
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                candidates.push((name, entry.path(), size));
            }
        }
        if candidates.is_empty() {
            return;
        }
        // Largest packs first so binary search hits the compacted main pack first.
        candidates.sort_by(|a, b| b.2.cmp(&a.2));
        let mut opened = Vec::new();
        let mut names = Vec::new();
        for (name, path, _) in candidates {
            if let Some(pack) = Pack::open(&path, &self.abi) {
                opened.push(pack);
                names.push(name);
            }
        }
        if opened.is_empty() {
            return;
        }
        {
            let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
            for name in names {
                loaded.insert(name);
            }
        }
        if let Ok(mut packs) = self.packs.write() {
            packs.extend(opened);
            packs.sort_by(|a, b| b.entry_count.cmp(&a.entry_count));
        }
    }

    #[inline]
    pub(crate) fn get(&self, key: &EntryKey) -> Option<StoredBytes> {
        if let Ok(packs) = self.packs.read() {
            for pack in packs.iter() {
                if let Some(found) = pack.get(key) {
                    return Some(found);
                }
            }
        }
        if self.has_pending.load(Ordering::Acquire) {
            if let Ok(pending) = self.pending.lock() {
                if let Some(bytes) = pending.get(key) {
                    return Some(StoredBytes::owned(bytes.clone()));
                }
            }
        }
        None
    }

    pub(crate) fn put(&self, key: EntryKey, payload: Arc<[u8]>) {
        if let Ok(packs) = self.packs.read() {
            for pack in packs.iter() {
                if pack.get(&key).is_some() {
                    return;
                }
            }
        }
        if let Ok(mut pending) = self.pending.lock() {
            pending.entry(key).or_insert(payload);
            self.has_pending.store(true, Ordering::Release);
        }
    }

    /// Writes pending entries into a new content-addressed immutable `.mpack` file (atomic rename)
    /// and compacts if too many small delta packs have accumulated.
    pub(crate) fn flush(&self) {
        if !self.has_pending.load(Ordering::Acquire) {
            return;
        }
        let Ok(_guard) = self.flush_lock.lock() else { return };
        let pending = {
            let Ok(mut map) = self.pending.lock() else { return };
            if map.is_empty() {
                self.has_pending.store(false, Ordering::Release);
                return;
            }
            self.has_pending.store(false, Ordering::Release);
            std::mem::take(&mut *map)
        };
        // Pick up any pack another process just published before deciding what is still missing.
        self.refresh_from_disk();
        let mut to_write: Vec<(EntryKey, Arc<[u8]>)> = {
            let Ok(packs) = self.packs.read() else { return };
            pending
                .into_iter()
                .filter(|(key, _)| !packs.iter().any(|p| p.get(key).is_some()))
                .collect()
        };
        if to_write.is_empty() {
            return;
        }
        to_write.sort_by_key(|(key, _)| *key);
        let slices: Vec<(EntryKey, &[u8])> = to_write.iter().map(|(k, v)| (*k, &**v)).collect();
        let Some(buf) = serialize_pack(&slices, &self.abi) else { return };
        if let Some(pack) = self.publish_pack(&buf) {
            let need_compact = {
                if let Ok(mut packs) = self.packs.write() {
                    packs.push(pack);
                    packs.sort_by(|a, b| b.entry_count.cmp(&a.entry_count));
                    packs.len() > COMPACT_THRESHOLD
                } else {
                    false
                }
            };
            if need_compact {
                self.compact_locked();
            }
        }
    }

    fn publish_pack(&self, buf: &[u8]) -> Option<Arc<Pack>> {
        if std::fs::create_dir_all(&self.packs_dir).is_err() {
            return None;
        }
        let digest = sha256(buf);
        let file_name = format!("{}.mpack", hex16(&digest));
        let target = self.packs_dir.join(&file_name);
        if !target.is_file() {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp_name = format!("{}.{}-{seq}.tmp", hex16(&digest), std::process::id());
            let tmp = self.packs_dir.join(tmp_name);
            if std::fs::write(&tmp, buf).is_err() {
                let _ = std::fs::remove_file(&tmp);
                return None;
            }
            if std::fs::rename(&tmp, &target).is_err() {
                let _ = std::fs::remove_file(&tmp);
                // Another process concurrently published the identical content-addressed pack.
                if !target.is_file() {
                    return None;
                }
            }
        }
        let pack = Pack::open(&target, &self.abi)?;
        if let Ok(mut loaded) = self.loaded.lock() {
            loaded.insert(OsString::from(file_name));
        }
        Some(pack)
    }

    fn compact_locked(&self) {
        self.refresh_from_disk();
        let snapshot: Vec<Arc<Pack>> = match self.packs.read() {
            Ok(packs) => packs.clone(),
            Err(_) => return,
        };
        if snapshot.len() <= 1 {
            return;
        }
        let mut merged: FxHashMap<EntryKey, &[u8]> = FxHashMap::default();
        for pack in &snapshot {
            for (key, payload) in pack.iter_entries() {
                merged.entry(key).or_insert(payload);
            }
        }
        let mut entries: Vec<(EntryKey, &[u8])> = merged.into_iter().collect();
        entries.sort_by_key(|(key, _)| *key);
        let Some(buf) = serialize_pack(&entries, &self.abi) else { return };
        let Some(new_pack) = self.publish_pack(&buf) else { return };
        let new_path = new_pack.path.clone();
        let old_paths: Vec<PathBuf> = snapshot.iter().filter(|p| p.path != new_path).map(|p| p.path.clone()).collect();
        // Drop our own mappings of the old packs before attempting to unlink them (Windows forbids
        // removing files while mapped in any process; other processes' mappings simply leave the
        // old file until a later compaction).
        if let Ok(mut packs) = self.packs.write() {
            *packs = vec![new_pack];
        }
        drop(snapshot);
        for old in old_paths {
            let _ = std::fs::remove_file(&old);
        }
        self.sweep_stale_temps();
    }

    fn sweep_stale_temps(&self) {
        let Ok(read_dir) = std::fs::read_dir(&self.packs_dir) else { return };
        let now = SystemTime::now();
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            if !name.to_str().is_some_and(|s| s.ends_with(".tmp")) {
                continue;
            }
            let stale = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age >= STALE_TMP_AGE);
            if stale {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Process-wide store registry keyed by resolved state directory.
static STORES: OnceLock<RwLock<FxHashMap<PathBuf, Arc<Store>>>> = OnceLock::new();
/// Single-entry fast path for the common case where every engine in a process shares one state dir.
static PRIMARY_STORE: OnceLock<(PathBuf, Arc<Store>)> = OnceLock::new();

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("REE_MODULE_CACHE").as_deref() != Ok("0"))
}

/// Versioned compiler ABI digest: hashes format version, crate version, target arch, float-flush
/// support, and the actual bytecode emitted by both `compiler()` and `loadmodule_compiler()` on a
/// canonical probe snippet so any compiler/Luau/flag change cleanly separates cache layouts.
pub(crate) fn abi_info() -> &'static ([u8; 32], String) {
    static ABI: OnceLock<([u8; 32], String)> = OnceLock::new();
    ABI.get_or_init(|| {
        const PROBE: &str = "local v = Vector3.new(1, 2, 3)\nlocal t = typeof(v)\nlocal sub = 2 ^ -1074\nlocal n = math.abs(-1) + os.clock()\nlocal s = script\nlocal r = require\nreturn { v, t, sub, n, s, r }\n";
        let mut material = Vec::with_capacity(512);
        material.extend_from_slice(PACK_MAGIC);
        material.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        material.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
        material.push(0);
        material.extend_from_slice(std::env::consts::ARCH.as_bytes());
        material.push(u8::from(super::super::float_mode::supported()));
        if let Ok(bc) = super::compiler().compile(PROBE) {
            material.extend_from_slice(&(bc.len() as u32).to_le_bytes());
            material.extend_from_slice(&bc);
        }
        material.push(0xff);
        if let Ok(bc) = super::loadmodule_compiler().compile(PROBE) {
            material.extend_from_slice(&(bc.len() as u32).to_le_bytes());
            material.extend_from_slice(&bc);
        }
        let digest = sha256(&material);
        let tag = format!("v{FORMAT_VERSION}-{}", hex8(&digest));
        (digest, tag)
    })
}

pub(crate) fn for_state_dir(state_dir: &Path) -> Option<Arc<Store>> {
    if !enabled() {
        return None;
    }
    if let Some((primary_path, primary)) = PRIMARY_STORE.get() {
        if primary_path == state_dir {
            return Some(primary.clone());
        }
    }
    let map = STORES.get_or_init(|| RwLock::new(FxHashMap::default()));
    if let Some(found) = map.read().ok().and_then(|m| m.get(state_dir).cloned()) {
        return Some(found);
    }
    let (abi, tag) = abi_info();
    let versioned = state_dir.join("module-store").join(tag);
    let created = Store::open_at(versioned, *abi);
    let _ = PRIMARY_STORE.set((state_dir.to_path_buf(), created.clone()));
    if let Ok(mut guard) = map.write() {
        let entry = guard.entry(state_dir.to_path_buf()).or_insert_with(|| created.clone());
        return Some(entry.clone());
    }
    Some(created)
}

pub(crate) fn default_store() -> Option<Arc<Store>> {
    if !enabled() {
        return None;
    }
    let state = crate::try_state_dir(&crate::default_repo()).ok()?;
    for_state_dir(&state)
}

pub(crate) fn for_engine(engine: &crate::Engine) -> Option<Arc<Store>> {
    if !enabled() {
        return None;
    }
    let state = crate::try_state_dir(&engine.options.repo).ok()?;
    for_state_dir(&state)
}

pub(crate) fn flush_all() {
    if let Some((_, primary)) = PRIMARY_STORE.get() {
        primary.flush();
    }
    if let Some(map) = STORES.get() {
        let list: Vec<Arc<Store>> = map.read().map(|m| m.values().cloned().collect()).unwrap_or_default();
        for store in list {
            store.flush();
        }
    }
}

/// Content hash key for a source buffer and artifact kind.
#[inline]
pub(crate) fn key_for(kind: u8, source: &[u8]) -> EntryKey {
    EntryKey { kind, hash: sha256(source), source_len: source.len() as u64 }
}

/// SHA-256 digest of `bytes`. Uses `sha2` when feature `bundles` is enabled and a matching
/// pure-Rust implementation otherwise, keeping zero extra dependencies.
#[cfg(feature = "bundles")]
pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

#[cfg(not(feature = "bundles"))]
pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((bytes.len() + 9).div_ceil(64) * 64);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    let mut w = [0u32; 64];
    for chunk in padded.chunks_exact(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn hex8(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(16);
    for b in &bytes[..8] {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn hex16(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(32);
    for b in &bytes[..16] {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

// --------------------------------------------------------------------------- Compact Binary JSON

fn write_varint(out: &mut Vec<u8>, mut value: usize) {
    while value >= 0x80 {
        out.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

#[inline]
fn read_varint_checked(bytes: &[u8], pos: &mut usize) -> Option<usize> {
    let mut result = 0usize;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        if shift >= usize::BITS {
            return None;
        }
        result |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

#[inline]
unsafe fn read_varint_unchecked(bytes: &[u8], pos: &mut usize) -> usize {
    let mut result = 0usize;
    let mut shift = 0u32;
    loop {
        let byte = unsafe { *bytes.get_unchecked(*pos) };
        *pos += 1;
        result |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return result;
        }
        shift += 7;
    }
}

/// Encodes a parsed `serde_json::Value` into a compact, sequential binary representation preserving
/// exact key order (`BTreeMap`), float bits, array capacity/sparse nulls, and object capacity.
pub(crate) fn encode_bjson(value: &serde_json::Value) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(BJSON_MAGIC);
    encode_bjson_value(value, &mut out);
    out
}

fn encode_bjson_value(value: &serde_json::Value, out: &mut Vec<u8>) {
    match value {
        serde_json::Value::Null => out.push(TAG_NULL),
        serde_json::Value::Bool(false) => out.push(TAG_FALSE),
        serde_json::Value::Bool(true) => out.push(TAG_TRUE),
        serde_json::Value::Number(num) => {
            out.push(TAG_NUMBER);
            out.extend_from_slice(&num.as_f64().unwrap_or(0.0).to_le_bytes());
        }
        serde_json::Value::String(text) => {
            out.push(TAG_STRING);
            write_varint(out, text.len());
            out.extend_from_slice(text.as_bytes());
        }
        serde_json::Value::Array(items) => {
            out.push(TAG_ARRAY);
            write_varint(out, items.len());
            for item in items {
                encode_bjson_value(item, out);
            }
        }
        serde_json::Value::Object(entries) => {
            out.push(TAG_OBJECT);
            write_varint(out, entries.len());
            let non_null = entries.values().filter(|v| !v.is_null()).count();
            write_varint(out, non_null);
            for (key, item) in entries {
                if !item.is_null() {
                    write_varint(out, key.len());
                    out.extend_from_slice(key.as_bytes());
                    encode_bjson_value(item, out);
                }
            }
        }
    }
}

/// Validates a `BinaryJson` buffer once (magic, bounds, exact trailing length, and max depth) so
/// subsequent raw C-API decodes are guaranteed memory-safe without per-byte error paths.
pub(crate) fn validate_bjson(bytes: &[u8]) -> bool {
    if bytes.len() < BJSON_MAGIC.len() || &bytes[..4] != BJSON_MAGIC {
        return false;
    }
    let mut pos = 4;
    validate_bjson_node(bytes, &mut pos, 0) && pos == bytes.len()
}

fn validate_bjson_node(bytes: &[u8], pos: &mut usize, depth: usize) -> bool {
    if depth > MAX_JSON_DEPTH {
        return false;
    }
    let Some(&tag) = bytes.get(*pos) else { return false };
    *pos += 1;
    match tag {
        TAG_NULL | TAG_FALSE | TAG_TRUE => true,
        TAG_NUMBER => {
            let Some(next) = pos.checked_add(8) else { return false };
            if next > bytes.len() {
                return false;
            }
            *pos = next;
            true
        }
        TAG_STRING => {
            let Some(len) = read_varint_checked(bytes, pos) else { return false };
            let Some(next) = pos.checked_add(len) else { return false };
            if next > bytes.len() {
                return false;
            }
            *pos = next;
            true
        }
        TAG_ARRAY => {
            let Some(len) = read_varint_checked(bytes, pos) else { return false };
            for _ in 0..len {
                if !validate_bjson_node(bytes, pos, depth + 1) {
                    return false;
                }
            }
            true
        }
        TAG_OBJECT => {
            let Some(_cap) = read_varint_checked(bytes, pos) else { return false };
            let Some(count) = read_varint_checked(bytes, pos) else { return false };
            for _ in 0..count {
                let Some(klen) = read_varint_checked(bytes, pos) else { return false };
                let Some(next) = pos.checked_add(klen) else { return false };
                if next > bytes.len() {
                    return false;
                }
                *pos = next;
                if !validate_bjson_node(bytes, pos, depth + 1) {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

/// Decodes a validated `BinaryJson` slice directly onto the Luau C stack in a single `exec_raw`
/// call, producing fresh mutable Luau tables with zero intermediate `mlua` registry refs.
pub(crate) fn bjson_to_lua(lua: &Lua, bytes: &[u8]) -> Result<LuaValue, String> {
    if bytes.len() < 4 || &bytes[..4] != BJSON_MAGIC {
        return Err("invalid binary JSON payload".into());
    }
    // SAFETY: `bytes` was verified by `validate_bjson` before entering the store or cache.
    unsafe {
        lua.exec_raw::<LuaValue>((), |state| {
            let mut pos = 4usize;
            push_bjson_node(state, bytes, &mut pos);
        })
    }
    .map_err(|error| error.to_string())
}

unsafe fn push_bjson_node(state: *mut ffi::lua_State, bytes: &[u8], pos: &mut usize) {
    unsafe {
        let tag = *bytes.get_unchecked(*pos);
        *pos += 1;
        match tag {
            TAG_NULL => ffi::lua_pushnil(state),
            TAG_FALSE => ffi::lua_pushboolean(state, 0),
            TAG_TRUE => ffi::lua_pushboolean(state, 1),
            TAG_NUMBER => {
                let ptr = bytes.as_ptr().add(*pos);
                *pos += 8;
                let mut raw = [0u8; 8];
                std::ptr::copy_nonoverlapping(ptr, raw.as_mut_ptr(), 8);
                ffi::lua_pushnumber(state, f64::from_le_bytes(raw));
            }
            TAG_STRING => {
                let len = read_varint_unchecked(bytes, pos);
                let ptr = bytes.as_ptr().add(*pos) as *const c_char;
                *pos += len;
                ffi::lua_pushlstring_(state, ptr, len);
            }
            TAG_ARRAY => {
                ffi::lua_rawcheckstack(state, 4);
                let len = read_varint_unchecked(bytes, pos);
                ffi::lua_createtable(state, len.min(c_int::MAX as usize) as c_int, 0);
                for index in 0..len {
                    if *bytes.get_unchecked(*pos) == TAG_NULL {
                        *pos += 1;
                    } else {
                        push_bjson_node(state, bytes, pos);
                        ffi::lua_rawseti_(state, -2, (index + 1) as c_int);
                    }
                }
            }
            TAG_OBJECT => {
                ffi::lua_rawcheckstack(state, 4);
                let cap = read_varint_unchecked(bytes, pos);
                let count = read_varint_unchecked(bytes, pos);
                ffi::lua_createtable(state, 0, cap.min(c_int::MAX as usize) as c_int);
                for _ in 0..count {
                    let klen = read_varint_unchecked(bytes, pos);
                    let kptr = bytes.as_ptr().add(*pos) as *const c_char;
                    *pos += klen;
                    ffi::lua_pushlstring_(state, kptr, klen);
                    push_bjson_node(state, bytes, pos);
                    ffi::lua_rawset(state, -3);
                }
            }
            _ => ffi::lua_pushnil(state),
        }
    }
}

/// Direct raw C-API converter from `serde_json::Value` to a fresh Luau value (no intermediate mlua
/// ref churn for nested tables/strings).
pub(crate) fn serde_json_to_lua(lua: &Lua, value: &serde_json::Value) -> Result<LuaValue, String> {
    unsafe {
        lua.exec_raw::<LuaValue>((), |state| {
            push_serde_json_node(state, value);
        })
    }
    .map_err(|error| error.to_string())
}

unsafe fn push_serde_json_node(state: *mut ffi::lua_State, value: &serde_json::Value) {
    unsafe {
        match value {
            serde_json::Value::Null => ffi::lua_pushnil(state),
            serde_json::Value::Bool(flag) => ffi::lua_pushboolean(state, c_int::from(*flag)),
            serde_json::Value::Number(num) => ffi::lua_pushnumber(state, num.as_f64().unwrap_or(0.0)),
            serde_json::Value::String(text) => ffi::lua_pushlstring_(state, text.as_ptr() as *const c_char, text.len()),
            serde_json::Value::Array(items) => {
                ffi::lua_rawcheckstack(state, 4);
                ffi::lua_createtable(state, items.len().min(c_int::MAX as usize) as c_int, 0);
                for (index, item) in items.iter().enumerate() {
                    if !item.is_null() {
                        push_serde_json_node(state, item);
                        ffi::lua_rawseti_(state, -2, (index + 1) as c_int);
                    }
                }
            }
            serde_json::Value::Object(entries) => {
                ffi::lua_rawcheckstack(state, 4);
                ffi::lua_createtable(state, 0, entries.len().min(c_int::MAX as usize) as c_int);
                for (key, item) in entries {
                    if !item.is_null() {
                        ffi::lua_pushlstring_(state, key.as_ptr() as *const c_char, key.len());
                        push_serde_json_node(state, item);
                        ffi::lua_rawset(state, -3);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ree-modstore-{label}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sha256_matches_standard_vectors() {
        let empty = sha256(b"");
        let empty_hex: String = empty.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(empty_hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let abc = sha256(b"abc");
        let abc_hex: String = abc.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(abc_hex, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn store_survives_half_written_temp_and_truncated_pack_files() {
        let dir = temp_dir("crash");
        let (abi, _) = abi_info();
        let store = Store::open_at(dir.clone(), *abi);
        let key = key_for(KIND_STD_BYTECODE, b"return 42");
        let payload: Arc<[u8]> = Arc::from(&b"compiled-bytes-42"[..]);
        store.put(key, payload.clone());
        store.flush();

        // Simulate a writer that died halfway through writing a temp file and a corrupt pack.
        let packs_dir = dir.join("packs");
        std::fs::write(packs_dir.join("deadwriter.999-0.tmp"), b"REEMPK01\x01\x00\x00").unwrap();
        std::fs::write(packs_dir.join("00000000000000000000000000000000.mpack"), b"REEMPK01truncated").unwrap();

        let reader = Store::open_at(dir.clone(), *abi);
        let loaded = reader.get(&key).expect("valid pack still readable beside torn files");
        assert_eq!(&*loaded, &*payload);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_survives_concurrent_writers_of_same_key() {
        let dir = temp_dir("race");
        let (abi, _) = abi_info();
        let shared_key = key_for(KIND_STD_BYTECODE, b"return 'shared'");
        let shared_payload: Arc<[u8]> = Arc::from(&b"shared-bytecode-payload"[..]);
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let dir = dir.clone();
                let abi = *abi;
                let shared_payload = shared_payload.clone();
                std::thread::spawn(move || {
                    let store = Store::open_at(dir, abi);
                    store.put(shared_key, shared_payload);
                    let own_src = format!("return {i}");
                    let own_key = key_for(KIND_STD_BYTECODE, own_src.as_bytes());
                    store.put(own_key, Arc::from(format!("bc-{i}").into_bytes()));
                    store.flush();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let reader = Store::open_at(dir.clone(), *abi);
        assert_eq!(&*reader.get(&shared_key).unwrap(), &*shared_payload);
        for i in 0..8 {
            let own_src = format!("return {i}");
            let own_key = key_for(KIND_STD_BYTECODE, own_src.as_bytes());
            assert_eq!(&*reader.get(&own_key).unwrap(), format!("bc-{i}").as_bytes());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_rejects_older_layout_version_and_mismatched_abi() {
        let dir = temp_dir("version");
        let (abi, _) = abi_info();
        let packs_dir = dir.join("packs");
        std::fs::create_dir_all(&packs_dir).unwrap();

        // 1) Wrong ABI in header
        let mut wrong_abi = *abi;
        wrong_abi[0] ^= 0xff;
        let key = key_for(KIND_STD_BYTECODE, b"return 1");
        let bad_abi_buf = serialize_pack(&[(key, b"stale")], &wrong_abi).unwrap();
        std::fs::write(packs_dir.join("11111111111111111111111111111111.mpack"), &bad_abi_buf).unwrap();

        // 2) Older format version (0 instead of FORMAT_VERSION)
        let mut old_ver_buf = serialize_pack(&[(key, b"old-format")], abi).unwrap();
        old_ver_buf[8..12].copy_from_slice(&0u32.to_le_bytes());
        std::fs::write(packs_dir.join("22222222222222222222222222222222.mpack"), &old_ver_buf).unwrap();

        let reader = Store::open_at(dir.clone(), *abi);
        assert!(reader.get(&key).is_none(), "incompatible ABI and old format version must not load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_immediately_after_build_invalidates_by_content_even_with_same_length() {
        let dir = temp_dir("edit");
        let (abi, _) = abi_info();
        let src_file = dir.join("Mod.luau");
        std::fs::write(&src_file, "return 'aaa1'").unwrap();
        let bytes1 = std::fs::read(&src_file).unwrap();
        let bc1 = super::super::compiler().compile(&bytes1).unwrap();
        let store = Store::open_at(dir.join("store"), *abi);
        store.put(key_for(KIND_STD_BYTECODE, &bytes1), Arc::from(bc1.as_slice()));
        store.flush();

        // Overwrite immediately with identical byte length (can share same OS mtime tick).
        std::fs::write(&src_file, "return 'bbb2'").unwrap();
        let bytes2 = std::fs::read(&src_file).unwrap();
        assert_eq!(bytes1.len(), bytes2.len());
        let reader = Store::open_at(dir.join("store"), *abi);
        assert!(reader.get(&key_for(KIND_STD_BYTECODE, &bytes2)).is_none(), "same-length edit must miss old content key");
        assert!(reader.get(&key_for(KIND_STD_BYTECODE, &bytes1)).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compacts_many_delta_packs_preserving_all_entries() {
        let dir = temp_dir("compact");
        let (abi, _) = abi_info();
        let store = Store::open_at(dir.clone(), *abi);
        for i in 0..=COMPACT_THRESHOLD {
            let src = format!("return {i}");
            store.put(key_for(KIND_STD_BYTECODE, src.as_bytes()), Arc::from(format!("bc-{i}").into_bytes()));
            store.flush();
        }
        assert_eq!(store.packs.read().unwrap().len(), 1, "expected compaction right after COMPACT_THRESHOLD + 1 packs");
        for i in (COMPACT_THRESHOLD + 1)..(COMPACT_THRESHOLD + 4) {
            let src = format!("return {i}");
            store.put(key_for(KIND_STD_BYTECODE, src.as_bytes()), Arc::from(format!("bc-{i}").into_bytes()));
            store.flush();
        }
        assert!(store.packs.read().unwrap().len() <= COMPACT_THRESHOLD);
        let reader = Store::open_at(dir.clone(), *abi);
        for i in 0..(COMPACT_THRESHOLD + 4) {
            let src = format!("return {i}");
            let got = reader.get(&key_for(KIND_STD_BYTECODE, src.as_bytes())).unwrap();
            assert_eq!(&*got, format!("bc-{i}").as_bytes());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bjson_roundtrips_arrays_sparse_nulls_objects_and_primitives() {
        let raw = r#"{"a": 1.5, "b": true, "c": false, "d": null, "e": "hello", "f": [10, null, 30], "g": {"nested": [1, 2], "empty": {}}, "h": []}"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let encoded = encode_bjson(&parsed);
        assert!(validate_bjson(&encoded));
        assert!(!validate_bjson(&encoded[..encoded.len() - 1]));

        let lua = Lua::new();
        let val = bjson_to_lua(&lua, &encoded).unwrap();
        let tbl = match val {
            LuaValue::Table(t) => t,
            other => panic!("expected table, got {other:?}"),
        };
        assert_eq!(tbl.get::<f64>("a").unwrap(), 1.5);
        assert!(tbl.get::<bool>("b").unwrap());
        assert!(!tbl.get::<bool>("c").unwrap());
        assert!(matches!(tbl.get::<LuaValue>("d").unwrap(), LuaValue::Nil));
        assert_eq!(tbl.get::<String>("e").unwrap(), "hello");
        let arr: LuaTable = tbl.get("f").unwrap();
        assert_eq!(arr.get::<f64>(1).unwrap(), 10.0);
        assert!(matches!(arr.get::<LuaValue>(2).unwrap(), LuaValue::Nil));
        assert_eq!(arr.get::<f64>(3).unwrap(), 30.0);
        let g: LuaTable = tbl.get("g").unwrap();
        let nested: LuaTable = g.get("nested").unwrap();
        assert_eq!(nested.get::<f64>(2).unwrap(), 2.0);
    }
}
