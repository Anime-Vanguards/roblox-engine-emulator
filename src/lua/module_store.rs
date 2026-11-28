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
//! - Writers serialize to a unique temporary file beside the target, flush it to disk, and atomically
//!   rename it into a content-addressed `.mpack` name; a crashed writer leaves an ignored `.tmp` file,
//!   and concurrent writers of the same pack converge on the identical immutable file without blocking.
//! - Readers validate magic, format version, ABI digest, bounds, and index ordering on open,
//!   silently ignoring incompatible or corrupted files, and check each payload against its record's
//!   checksum before handing it out: a payload that reads otherwise is a miss. Windows can show a
//!   just-written file's pages zero-filled through a mapping while the machine is under heavy write
//!   load (a cold `golden --all`: whole 256 KB runs of a fresh pack read as zeros in the writer and in
//!   other processes), and Luau reports bytecode that starts with a zero byte as an error whose text is
//!   only the chunk name.
//! - One process compacts at a time (a try-lock on `compact.lock`; the others skip), and packs that a
//!   larger pack already holds entirely (a compaction's inputs that a sibling still had mapped, so they
//!   could not be removed) are dropped from the lookup list and never count toward compaction.

use std::{
    ffi::{OsString, c_char, c_int},
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use mlua::{ffi, prelude::*};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::project::plan::Mapping;

pub(crate) const KIND_STD_BYTECODE: u8 = 1;
pub(crate) const KIND_LOADMODULE_BYTECODE: u8 = 2;
/// 4: a JSON module read as `rojo build` 7.6.1 reads it (project/data_files.rs: lenient JSONC, and a surrogate escape, a NUL
/// or deep nesting no longer read as strict JSON's value), so no entry 3, of strict JSON, is read again
pub(crate) const KIND_BINARY_JSON: u8 = 4;

const PACK_MAGIC: &[u8; 8] = b"REEMPK01";
/// 2: each record carries its payload's [`payload_check`]
const FORMAT_VERSION: u32 = 2;
const HEADER_LEN: usize = 48;
const RECORD_LEN: usize = 64;
/// Packs (beyond those a larger pack holds entirely) a store keeps before it compacts them into one: a cold
/// `golden --all` (a dozen batch workers, each flushing after every job) publishes a delta pack per job, and each
/// compaction rewrites every entry (40 MB for rur's frozen project)
const COMPACT_THRESHOLD: usize = 16;
/// The file whose lock a compacting process holds (`File::try_lock`): a process that cannot take it skips
const COMPACT_LOCK: &str = "compact.lock";
/// The file whose lock serializes cross-process delta refresh-filter-publish (`try_lock` for deferred
/// flushes, blocking `lock` for final flushes), independent of [`COMPACT_LOCK`].
const FLUSH_LOCK: &str = "flush.lock";
/// The entry count up to which [`Pack::covers`] reads the covering pack's payloads, not only its records
const COVER_PAYLOADS_UP_TO: usize = 64;
const STALE_TMP_AGE: Duration = Duration::from_secs(300);
/// Pending entries or payload bytes a long-lived batch worker accumulates before publishing a delta pack.
const DEFERRED_FLUSH_ENTRIES: usize = 256;
const DEFERRED_FLUSH_BYTES: usize = 4 * 1024 * 1024;
/// Minimum interval between `packs/` mtime checks on cold misses so sibling workers pick up fresh packs cheaply.
const MISS_REFRESH_INTERVAL_MS: u64 = 500;

static DEFERRED_FLUSH: AtomicBool = AtomicBool::new(false);
/// True only during in-process `ree run --batch` where all worker threads share one `Store` and flush
/// once at batch end. Kept separate from `is_batch_frozen()` so multi-process frozen batch workers
/// (such as `rur-native engine-batch --frozen`) still run deferred flushes mid-run.
static SUPPRESS_PER_RUN_FLUSH: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_deferred_flush(defer: bool) {
    DEFERRED_FLUSH.store(defer, Ordering::Relaxed);
}

pub(crate) fn set_suppress_per_run_flush(suppress: bool) {
    SUPPRESS_PER_RUN_FLUSH.store(suppress, Ordering::Relaxed);
}

fn elapsed_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

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
            let (key, offset, len, _) = read_record(bytes, index);
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

    /// The record of `target`: (payload offset, length, checksum).
    #[inline]
    fn find(&self, target: &EntryKey) -> Option<(usize, usize, u64)> {
        let bytes = self.backing.bytes();
        let mut lo = 0usize;
        let mut hi = self.entry_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let (key, offset, len, check) = read_record(bytes, mid);
            match key.cmp(target) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some((offset, len, check)),
            }
        }
        None
    }

    /// `target`'s payload, when its bytes read as they were stored (else None, as for a missing entry).
    #[inline]
    fn get(&self, target: &EntryKey) -> Option<StoredBytes> {
        let (offset, len, check) = self.find(target)?;
        let payload = &self.backing.bytes()[offset..offset + len];
        if payload_check(payload) != check {
            DAMAGED.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        Some(StoredBytes { backing: self.backing.clone(), start: offset, end: offset + len })
    }

    /// Whether this pack holds every entry `other` has, as `other` stored it (the same checksum in both records). Only the
    /// record tables are read for a large pack: reading every payload put each new compacted pack's 40 MB into every
    /// worker's working set (a cold `golden --all`: 1.5 GB at a worker's peak, against 350 MB before). For a small pack,
    /// such as the delta that replaced an entry whose copy here read back damaged, this pack's payloads are checked too,
    /// so that delta is kept while the copy here reads otherwise.
    fn covers(&self, other: &Pack) -> bool {
        let bytes = other.backing.bytes();
        let read_payloads = other.entry_count <= COVER_PAYLOADS_UP_TO;
        (0..other.entry_count).all(|index| {
            let (key, _, _, check) = read_record(bytes, index);
            self.find(&key).is_some_and(|(offset, len, own)| own == check && (!read_payloads || payload_check(&self.backing.bytes()[offset..offset + len]) == own))
        })
    }

    /// The entries whose payloads read as they were stored, with their checksums.
    fn iter_entries(&self) -> impl Iterator<Item = (EntryKey, &[u8], u64)> {
        let bytes = self.backing.bytes();
        (0..self.entry_count).filter_map(move |index| {
            let (key, offset, len, check) = read_record(bytes, index);
            let payload = &bytes[offset..offset + len];
            if payload_check(payload) != check {
                DAMAGED.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            Some((key, payload, check))
        })
    }
}

/// Payloads read back with other bytes than their checksum says (left out as misses)
static DAMAGED: AtomicU64 = AtomicU64::new(0);

/// How many payloads this process found damaged so far (and treated as missing).
pub(crate) fn damaged_reads() -> u64 {
    DAMAGED.load(Ordering::Relaxed)
}

/// A payload's checksum, kept in its record: 64 bits over its bytes, word by word (a payload that reads otherwise, such as a
/// zero-filled run of pages, fails it). Fixed here rather than borrowed from a hasher crate, so a dependency update never
/// changes what a pack on disk means.
fn payload_check(bytes: &[u8]) -> u64 {
    const MUL: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut hash = (bytes.len() as u64).wrapping_mul(MUL) ^ 0x243f_6a88_85a3_08d3;
    let mut words = bytes.chunks_exact(8);
    for word in &mut words {
        hash = (hash ^ u64::from_le_bytes(word.try_into().unwrap())).wrapping_mul(MUL).rotate_left(29);
    }
    let mut tail = [0u8; 8];
    tail[..words.remainder().len()].copy_from_slice(words.remainder());
    hash = (hash ^ u64::from_le_bytes(tail)).wrapping_mul(MUL);
    hash ^ (hash >> 32)
}

#[inline]
fn read_record(bytes: &[u8], index: usize) -> (EntryKey, usize, usize, u64) {
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
    let check = u64::from_le_bytes(bytes[base + 56..base + 64].try_into().unwrap());
    (EntryKey { kind, hash, source_len }, payload_offset, payload_len, check)
}

/// A pack's bytes: `entries` (sorted by key), each with its payload's [`payload_check`].
fn serialize_pack(entries: &[(EntryKey, &[u8], u64)], abi: &[u8; 32]) -> Option<Vec<u8>> {
    let count = u32::try_from(entries.len()).ok()?;
    let table_end = HEADER_LEN + entries.len() * RECORD_LEN;
    let payload_total: usize = entries.iter().map(|(_, p, _)| p.len()).sum();
    let mut buf = Vec::with_capacity(table_end + payload_total);
    buf.extend_from_slice(PACK_MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(abi);
    let mut offset = table_end as u64;
    for (key, payload, check) in entries {
        let len = u32::try_from(payload.len()).ok()?;
        buf.push(key.kind);
        buf.extend_from_slice(&[0u8; 3]);
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&key.hash);
        buf.extend_from_slice(&key.source_len.to_le_bytes());
        buf.extend_from_slice(&offset.to_le_bytes());
        buf.extend_from_slice(&check.to_le_bytes());
        offset += payload.len() as u64;
    }
    for (_, payload, _) in entries {
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
    pending_entries: AtomicUsize,
    pending_bytes: AtomicUsize,
    has_pending: AtomicBool,
    last_dir_modified: AtomicU64,
    last_miss_probe_ms: AtomicU64,
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
            pending_entries: AtomicUsize::new(0),
            pending_bytes: AtomicUsize::new(0),
            has_pending: AtomicBool::new(false),
            last_dir_modified: AtomicU64::new(0),
            last_miss_probe_ms: AtomicU64::new(elapsed_ms()),
            flush_lock: Mutex::new(()),
        });
        store.refresh_from_disk_locked();
        store
    }

    /// Scans `packs/` for newly published `.mpack` files and adds them to `packs`.
    /// Callers must either hold `self.flush_lock` or have exclusive access during `open_at`.
    fn refresh_from_disk_locked(&self) -> bool {
        if let Some(stamp) = crate::files::stamp(&self.packs_dir) {
            self.last_dir_modified.store(stamp.modified, Ordering::Relaxed);
        }
        let Ok(read_dir) = std::fs::read_dir(&self.packs_dir) else { return false };
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
            return false;
        }
        // Largest packs first so binary search hits the compacted main pack first.
        candidates.sort_by(|a, b| b.2.cmp(&a.2));
        let mut opened = Vec::new();
        for (name, path, _) in candidates {
            if let Some(pack) = Pack::open(&path, &self.abi) {
                opened.push((name, pack));
            }
        }
        if opened.is_empty() {
            return false;
        }
        let mut added = false;
        let pruned = match self.packs.write() {
            Ok(mut packs) => {
                let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
                for (name, pack) in opened {
                    loaded.insert(name);
                    if !packs.iter().any(|existing| existing.path == pack.path) {
                        packs.push(pack);
                        added = true;
                    }
                }
                drop(loaded);
                prune_covered(&mut packs)
            }
            Err(_) => Vec::new(),
        };
        remove_packs(pruned);
        added
    }

    fn maybe_refresh_on_miss(&self) -> bool {
        let now = elapsed_ms();
        let last = self.last_miss_probe_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) < MISS_REFRESH_INTERVAL_MS {
            return false;
        }
        // Serialize miss-refresh against in-process flushes and compactions without ever stalling
        // callers if a flush or compaction is already underway.
        let Ok(_guard) = self.flush_lock.try_lock() else { return false };
        let last = self.last_miss_probe_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) < MISS_REFRESH_INTERVAL_MS {
            return false;
        }
        self.last_miss_probe_ms.store(now, Ordering::Relaxed);
        let Some(stamp) = crate::files::stamp(&self.packs_dir) else { return false };
        if stamp.modified == self.last_dir_modified.load(Ordering::Relaxed) {
            return false;
        }
        self.refresh_from_disk_locked()
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
        if self.maybe_refresh_on_miss() {
            if let Ok(packs) = self.packs.read() {
                for pack in packs.iter() {
                    if let Some(found) = pack.get(key) {
                        return Some(found);
                    }
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
            if let std::collections::hash_map::Entry::Vacant(slot) = pending.entry(key) {
                let len = payload.len();
                slot.insert(payload);
                self.pending_entries.fetch_add(1, Ordering::Relaxed);
                self.pending_bytes.fetch_add(len, Ordering::Relaxed);
                self.has_pending.store(true, Ordering::Release);
            }
        }
    }

    /// Flushes only once enough pending entries or bytes have accumulated (used between jobs of a
    /// long-lived batch worker to avoid publishing a tiny delta pack per job).
    pub(crate) fn flush_deferred(&self) {
        self.flush_inner(true);
    }

    /// Writes pending entries into a new content-addressed immutable `.mpack` file (atomic rename)
    /// and compacts if too many small delta packs have accumulated.
    pub(crate) fn flush(&self) {
        self.flush_inner(false);
    }

    fn flush_inner(&self, defer: bool) {
        if !self.has_pending.load(Ordering::Acquire) {
            return;
        }
        if defer
            && self.pending_entries.load(Ordering::Relaxed) < DEFERRED_FLUSH_ENTRIES
            && self.pending_bytes.load(Ordering::Relaxed) < DEFERRED_FLUSH_BYTES
        {
            return;
        }
        let Ok(_guard) = self.flush_lock.lock() else { return };
        // Serialize refresh-filter-publish across sibling processes on a dedicated flush lock
        // (distinct from COMPACT_LOCK so an in-progress compaction never blocks a delta flush) so
        // concurrent batch workers flushing overlapping cold compiles write each shared entry once
        // instead of leaving duplicate delta packs on disk. Deferred mid-run flushes use try_lock
        // so workers keep working while a peer publishes; final flushes wait so every unique entry
        // persists without duplicating peers' packs.
        let _ = std::fs::create_dir_all(&self.packs_dir);
        let cross_lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.packs_dir.join(FLUSH_LOCK))
            .ok();
        if let Some(ref lock) = cross_lock {
            let acquired = if defer { lock.try_lock().is_ok() } else { lock.lock().is_ok() };
            if !acquired {
                return;
            }
        }
        // Pick up any pack another process just published before deciding what is still missing.
        self.refresh_from_disk_locked();
        let mut to_write: Vec<(EntryKey, Arc<[u8]>)> = {
            let Ok(packs) = self.packs.read() else { return };
            let Ok(mut map) = self.pending.lock() else { return };
            map.retain(|key, _| !packs.iter().any(|p| p.get(key).is_some()));
            if map.is_empty() {
                self.has_pending.store(false, Ordering::Release);
                self.pending_entries.store(0, Ordering::Relaxed);
                self.pending_bytes.store(0, Ordering::Relaxed);
                return;
            }
            let remaining_bytes: usize = map.values().map(|v| v.len()).sum();
            if defer && map.len() < DEFERRED_FLUSH_ENTRIES && remaining_bytes < DEFERRED_FLUSH_BYTES {
                self.pending_entries.store(map.len(), Ordering::Relaxed);
                self.pending_bytes.store(remaining_bytes, Ordering::Relaxed);
                return;
            }
            self.has_pending.store(false, Ordering::Release);
            self.pending_entries.store(0, Ordering::Relaxed);
            self.pending_bytes.store(0, Ordering::Relaxed);
            std::mem::take(&mut *map).into_iter().collect()
        };
        to_write.sort_by_key(|(key, _)| *key);
        let entries: Vec<(EntryKey, &[u8], u64)> = to_write.iter().map(|(k, v)| (*k, &**v, payload_check(v))).collect();
        let Some(buf) = serialize_pack(&entries, &self.abi) else { return };
        let published = self.publish_pack(&buf);
        drop(cross_lock);
        if let Some(pack) = published {
            let (need_compact, pruned) = match self.packs.write() {
                Ok(mut packs) => {
                    if !packs.iter().any(|existing| existing.path == pack.path) {
                        packs.push(pack);
                    }
                    let pruned = prune_covered(&mut packs);
                    (packs.len() > COMPACT_THRESHOLD, pruned)
                }
                Err(_) => (false, Vec::new()),
            };
            remove_packs(pruned);
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
            if write_durably(&tmp, buf).is_err() {
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

    /// Merges this store's packs into one, unless another process is compacting (its pack will hold these entries) or
    /// already did (what it published leaves few enough packs).
    fn compact_locked(&self) {
        let Ok(lock) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.packs_dir.join(COMPACT_LOCK)) else { return };
        if lock.try_lock().is_err() {
            return;
        }
        self.refresh_from_disk_locked();
        let snapshot: Vec<Arc<Pack>> = match self.packs.read() {
            Ok(packs) => packs.clone(),
            Err(_) => return,
        };
        if snapshot.len() <= COMPACT_THRESHOLD {
            return;
        }
        self.compact_snapshot(snapshot);
        self.sweep_stale_temps();
        drop(lock);
    }

    /// Merges `snapshot` into one pack and drops only the packs in `snapshot` from `self.packs`,
    /// keeping any pack added outside `snapshot`.
    fn compact_snapshot(&self, snapshot: Vec<Arc<Pack>>) {
        // (only payloads that read as they were stored: a damaged copy is left out, or replaced by another pack's)
        let mut merged: FxHashMap<EntryKey, (&[u8], u64)> = FxHashMap::default();
        for pack in &snapshot {
            for (key, payload, check) in pack.iter_entries() {
                merged.entry(key).or_insert((payload, check));
            }
        }
        let mut entries: Vec<(EntryKey, &[u8], u64)> = merged.into_iter().map(|(key, (payload, check))| (key, payload, check)).collect();
        entries.sort_by_key(|(key, _, _)| *key);
        let Some(buf) = serialize_pack(&entries, &self.abi) else { return };
        drop(entries);
        let Some(new_pack) = self.publish_pack(&buf) else { return };
        let new_path = new_pack.path.clone();
        let old_paths: Vec<PathBuf> = snapshot.iter().filter(|p| p.path != new_path).map(|p| p.path.clone()).collect();
        let old_set: FxHashSet<PathBuf> = old_paths.iter().cloned().collect();
        // Drop only our mappings of the compacted snapshot packs before unlinking them, preserving
        // any pack loaded outside `snapshot`.
        let (extra_pruned, live): (Vec<PathBuf>, FxHashSet<PathBuf>) = match self.packs.write() {
            Ok(mut packs) => {
                packs.retain(|p| !old_set.contains(&p.path) && p.path != new_path);
                packs.push(new_pack);
                let extra_pruned = prune_covered(&mut packs);
                let live = packs.iter().map(|p| p.path.clone()).collect();
                (extra_pruned, live)
            }
            Err(_) => (Vec::new(), FxHashSet::default()),
        };
        drop(snapshot);
        for old in old_paths.into_iter().chain(extra_pruned) {
            if !live.contains(&old) {
                let _ = std::fs::remove_file(&old);
            }
        }
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

/// Sorts `packs` largest first and drops every other pack the largest holds entirely (a compaction's inputs: another
/// process's, or this one's that a sibling still had mapped so they could not be removed), so they are neither searched, nor
/// counted toward [`COMPACT_THRESHOLD`], nor kept mapped. Returns their paths.
fn prune_covered(packs: &mut Vec<Arc<Pack>>) -> Vec<PathBuf> {
    packs.sort_by(|a, b| b.entry_count.cmp(&a.entry_count));
    let Some(largest) = packs.first().cloned() else { return Vec::new() };
    let mut pruned = Vec::new();
    packs.retain(|pack| {
        let keep = Arc::ptr_eq(pack, &largest) || !largest.covers(pack);
        if !keep && pack.path != largest.path {
            pruned.push(pack.path.clone());
        }
        keep
    });
    pruned
}

/// Removes pack files no longer needed (best effort: one that another process still has mapped stays, and a later process
/// that finds it covered removes it).
fn remove_packs(paths: Vec<PathBuf>) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

/// Writes `bytes` to a new file at `path`, flushed to disk before it is renamed to the name readers open: what a reader
/// maps is then on disk whole, never pages whose writes are still pending.
fn write_durably(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
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
        // (`workspace.Gravity`: a field global's chain, which the compiler stopped importing: modules::FIELD_GLOBALS)
        const PROBE: &str = "local v = Vector3.new(1, 2, 3)\nlocal t = typeof(v)\nlocal sub = 2 ^ -1074\nlocal n = math.abs(-1) + os.clock()\nlocal s = script\nlocal r = require\nlocal g = workspace.Gravity\nreturn { v, t, sub, n, s, r, g }\n";
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

fn for_each_store(mut f: impl FnMut(&Store)) {
    if let Some((_, primary)) = PRIMARY_STORE.get() {
        f(primary);
    }
    if let Some(map) = STORES.get() {
        let list: Vec<Arc<Store>> = map.read().map(|m| m.values().cloned().collect()).unwrap_or_default();
        for store in list {
            f(&store);
        }
    }
}

pub(crate) fn flush_all() {
    if SUPPRESS_PER_RUN_FLUSH.load(Ordering::Relaxed) {
        // In-process `ree run --batch` shares one Store across all worker threads and flushes once
        // on batch completion so concurrent workers write a single pack with zero compaction churn.
        return;
    }
    if DEFERRED_FLUSH.load(Ordering::Relaxed) {
        for_each_store(|store| store.flush_deferred());
    } else {
        for_each_store(|store| store.flush());
    }
}

pub(crate) fn flush_all_now() {
    for_each_store(|store| store.flush());
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
        let bad_abi_buf = serialize_pack(&[(key, &b"stale"[..], payload_check(b"stale"))], &wrong_abi).unwrap();
        std::fs::write(packs_dir.join("11111111111111111111111111111111.mpack"), &bad_abi_buf).unwrap();

        // 2) Older format version (0 instead of FORMAT_VERSION)
        let mut old_ver_buf = serialize_pack(&[(key, &b"old-format"[..], payload_check(b"old-format"))], abi).unwrap();
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

    /// The payload a stress worker stores for entry `index`: deterministic, starting with a non-zero byte, sized so the
    /// packs grow to megabytes as a story's modules do.
    fn stress_payload(index: usize) -> Vec<u8> {
        let len = 2_000 + (index * 7919) % 30_000;
        let mut bytes = Vec::with_capacity(len);
        let mut state = index as u64 * 0x9e37_79b9_7f4a_7c15 + 1;
        bytes.push(6);
        while bytes.len() < len {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            bytes.push((state as u8) | 1);
        }
        bytes
    }

    fn stress_key(index: usize) -> EntryKey {
        key_for(KIND_STD_BYTECODE, format!("return {index} -- stress").as_bytes())
    }

    /// One stress worker (a child process): fills the store in rounds, as a batch worker does one job after another, and
    /// checks every entry it can read, its own and the other workers', after each round. Each round introduces a new
    /// slice of entries so even with `flush.lock` deduplication the run publishes more than `COMPACT_THRESHOLD` packs
    /// and exercises real cross-process compaction. Returns the bad reads.
    fn stress_worker(dir: &Path, worker: usize, entries: usize, rounds: usize) -> Vec<String> {
        let (abi, _) = abi_info();
        let store = Store::open_at(dir.to_path_buf(), *abi);
        let mut bad = Vec::new();
        let per_round = entries / rounds;
        for round in 0..rounds {
            let base = round * per_round;
            // Every worker writes overlapping entries inside this round's brand-new slice so
            // concurrent flushes contend on flush.lock while still advancing past COMPACT_THRESHOLD.
            for step in 0..per_round {
                let index = base + (worker * 7 + step) % per_round;
                let key = stress_key(index);
                if store.get(&key).is_none() {
                    store.put(key, Arc::from(stress_payload(index)));
                }
            }
            store.flush();
            let fresh = Store::open_at(dir.to_path_buf(), *abi);
            for index in 0..base + per_round {
                for (label, reader) in [("own", &store), ("fresh", &fresh)] {
                    if let Some(found) = reader.get(&stress_key(index)) {
                        if *found != *stress_payload(index) {
                            let zeros = found.iter().take(64).all(|&b| b == 0);
                            bad.push(format!("worker {worker} round {round} {label} entry {index}: {} bytes, first {:?}{}", found.len(), &found[..found.len().min(8)], if zeros { " (zeros)" } else { "" }));
                        }
                    }
                }
            }
        }
        bad
    }

    /// Many processes filling one empty store at once (a cold `golden --all`: 16 batch workers, each flushing after every
    /// job, compacting when deltas pile up) never read an entry back with other bytes than were stored.
    #[test]
    fn processes_filling_an_empty_store_at_once_read_back_what_was_stored() {
        const ENTRIES: usize = 600;
        const ROUNDS: usize = 24;
        if let Some(dir) = std::env::var_os("REE_MODSTORE_STRESS_DIR") {
            let worker: usize = std::env::var("REE_MODSTORE_STRESS_WORKER").unwrap().parse().unwrap();
            let bad = stress_worker(Path::new(&dir), worker, ENTRIES, ROUNDS);
            for line in &bad {
                println!("BAD {line}");
            }
            assert!(bad.is_empty(), "{} bad reads", bad.len());
            return;
        }
        let workers: usize = std::env::var("REE_MODSTORE_STRESS_WORKERS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
        let tries: usize = std::env::var("REE_MODSTORE_STRESS_TRIES").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let exe = std::env::current_exe().unwrap();
        let mut failures = Vec::new();
        for attempt in 0..tries {
            let dir = temp_dir(&format!("procs-{attempt}"));
            let children: Vec<_> = (0..workers)
                .map(|worker| {
                    std::process::Command::new(&exe)
                        .args(["lua::modules::store::tests::processes_filling_an_empty_store_at_once_read_back_what_was_stored", "--exact", "--nocapture", "--test-threads=1"])
                        .env("REE_MODSTORE_STRESS_DIR", &dir)
                        .env("REE_MODSTORE_STRESS_WORKER", worker.to_string())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .unwrap()
                })
                .collect();
            for child in children {
                let output = child.wait_with_output().unwrap();
                let text = String::from_utf8_lossy(&output.stdout);
                failures.extend(text.lines().filter(|line| line.starts_with("BAD ")).map(|line| format!("try {attempt}: {line}")));
                if !output.status.success() && !text.contains("BAD ") {
                    failures.push(format!("try {attempt}: a worker failed: {}{}", text, String::from_utf8_lossy(&output.stderr)));
                }
            }
            // Verify every entry persisted and compaction kept packs bounded below ROUNDS (> COMPACT_THRESHOLD).
            let (abi, _) = abi_info();
            let final_reader = Store::open_at(dir.clone(), *abi);
            for index in 0..ENTRIES {
                if final_reader.get(&stress_key(index)).as_deref() != Some(stress_payload(index).as_slice()) {
                    failures.push(format!("try {attempt}: final reader missing/corrupt entry {index}"));
                }
            }
            let remaining_packs = final_reader.packs.read().unwrap().len();
            if remaining_packs > COMPACT_THRESHOLD {
                failures.push(format!("try {attempt}: expected compaction across {ROUNDS} rounds (>{COMPACT_THRESHOLD}), got {remaining_packs} active packs"));
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert!(failures.is_empty(), "{} bad reads:\n{}", failures.len(), failures.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
    }

    /// The `.mpack` files in a store's packs folder.
    fn pack_files(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join("packs"))
            .map(|entries| entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext == "mpack")).collect())
            .unwrap_or_default();
        files.sort();
        files
    }

    /// A payload whose pages read back zero-filled (what a cold `golden --all` under heavy write load saw through a fresh
    /// pack's mapping: whole 256 KB runs of zeros, which Luau loads as an error whose text is only the chunk name) is a miss:
    /// the store hands out none of it, takes the payload again, and a compaction keeps the good copy.
    #[test]
    fn a_payload_that_reads_back_zero_filled_is_a_miss_and_is_stored_again() {
        let dir = temp_dir("zeroed");
        let (abi, _) = abi_info();
        let payloads: Vec<Vec<u8>> = (0..3).map(stress_payload).collect();
        {
            let store = Store::open_at(dir.clone(), *abi);
            for (index, payload) in payloads.iter().enumerate() {
                store.put(stress_key(index), Arc::from(payload.as_slice()));
            }
            store.flush();
        }
        // zero the middle payload where it sits in the pack file
        let file = pack_files(&dir).pop().unwrap();
        let mut bytes = std::fs::read(&file).unwrap();
        let at = bytes.windows(payloads[1].len()).position(|window| window == payloads[1].as_slice()).unwrap();
        bytes[at..at + payloads[1].len()].fill(0);
        std::fs::write(&file, &bytes).unwrap();

        let damaged_before = damaged_reads();
        let store = Store::open_at(dir.clone(), *abi);
        assert_eq!(store.get(&stress_key(0)).as_deref(), Some(payloads[0].as_slice()));
        assert!(store.get(&stress_key(1)).is_none(), "a zero-filled payload must read as missing, not as its zeros");
        assert_eq!(store.get(&stress_key(2)).as_deref(), Some(payloads[2].as_slice()));
        assert!(damaged_reads() > damaged_before);

        // the caller compiles again and stores the payload: the good copy goes into a new pack, and every later reader finds it
        store.put(stress_key(1), Arc::from(payloads[1].as_slice()));
        store.flush();
        assert_eq!(pack_files(&dir).len(), 2);
        assert_eq!(Store::open_at(dir.clone(), *abi).get(&stress_key(1)).as_deref(), Some(payloads[1].as_slice()));

        // a compaction merges the good copy, never the zeros
        // (with the first pack and the replacement, the last of these deltas makes one pack too many)
        for index in 3..2 + COMPACT_THRESHOLD {
            store.put(stress_key(index), Arc::from(stress_payload(index)));
            store.flush();
        }
        assert_eq!(store.packs.read().unwrap().len(), 1, "the deltas were compacted");
        let reader = Store::open_at(dir.clone(), *abi);
        for index in 0..2 + COMPACT_THRESHOLD {
            assert_eq!(reader.get(&stress_key(index)).as_deref(), Some(stress_payload(index).as_slice()), "entry {index}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Writes a pack holding `indices`' stress entries straight into `dir` (as another process would publish it).
    fn write_stress_pack(dir: &Path, indices: std::ops::Range<usize>) -> PathBuf {
        let (abi, _) = abi_info();
        let payloads: Vec<(EntryKey, Vec<u8>)> = indices.map(|index| (stress_key(index), stress_payload(index))).collect();
        let mut entries: Vec<(EntryKey, &[u8], u64)> = payloads.iter().map(|(key, payload)| (*key, payload.as_slice(), payload_check(payload))).collect();
        entries.sort_by_key(|(key, _, _)| *key);
        let buf = serialize_pack(&entries, abi).unwrap();
        let file = dir.join("packs").join(format!("{}.mpack", hex16(&sha256(&buf))));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, buf).unwrap();
        file
    }

    /// Packs that a larger pack holds entirely (a compaction's inputs that a sibling process still had mapped, so they could
    /// not be removed) are neither searched nor counted: a store that finds a compacted pack beside many of its old inputs
    /// does not compact again, and the old inputs it can remove are removed. Before, every worker of a cold `golden --all`
    /// counted them and rewrote the 40 MB pack after each job.
    #[test]
    fn packs_a_larger_pack_holds_are_not_counted_or_compacted_again() {
        let dir = temp_dir("covered");
        let (abi, _) = abi_info();
        let compacted = write_stress_pack(&dir, 0..200);
        for slice in 0..COMPACT_THRESHOLD + 4 {
            write_stress_pack(&dir, slice * 10..slice * 10 + 10);
        }
        assert_eq!(pack_files(&dir).len(), COMPACT_THRESHOLD + 5);

        let store = Store::open_at(dir.clone(), *abi);
        assert_eq!(store.packs.read().unwrap().len(), 1, "only the compacted pack is searched");
        assert_eq!(pack_files(&dir), vec![compacted.clone()], "the old inputs were removed");
        store.put(stress_key(500), Arc::from(stress_payload(500)));
        store.flush();
        let files = pack_files(&dir);
        assert_eq!(files.len(), 2, "one new delta, no new compaction: {files:?}");
        assert!(files.contains(&compacted));
        for index in (0..200).chain([500]) {
            assert_eq!(store.get(&stress_key(index)).as_deref(), Some(stress_payload(index).as_slice()), "entry {index}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A large pack is taken for covered by the records alone (its entries' keys and checksums in the larger pack: reading
    /// every payload of the larger pack put it whole into each process's working set), a small one only where the larger
    /// pack's payloads read back as stored, so the delta holding the good copy of an entry damaged there is kept.
    #[test]
    fn a_large_pack_is_covered_by_records_and_a_small_one_by_payloads_that_read_back() {
        let dir = temp_dir("cover-check");
        let (abi, _) = abi_info();
        let largest = write_stress_pack(&dir, 0..200);
        write_stress_pack(&dir, 0..150);
        let delta = write_stress_pack(&dir, 5..6);
        // zero two payloads in the largest pack, as the pages read back under write load
        let mut bytes = std::fs::read(&largest).unwrap();
        for index in [5, 100] {
            let payload = stress_payload(index);
            let at = bytes.windows(payload.len()).position(|window| window == payload.as_slice()).unwrap();
            bytes[at..at + payload.len()].fill(0);
        }
        std::fs::write(&largest, &bytes).unwrap();

        let store = Store::open_at(dir.clone(), *abi);
        let kept: Vec<PathBuf> = store.packs.read().unwrap().iter().map(|pack| pack.path.clone()).collect();
        assert_eq!(kept, vec![largest.clone(), delta.clone()], "the 150-entry pack is covered by its records; the delta is not");
        assert_eq!(store.get(&stress_key(5)).as_deref(), Some(stress_payload(5).as_slice()), "the delta's good copy");
        assert_eq!(store.get(&stress_key(6)).as_deref(), Some(stress_payload(6).as_slice()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// While one process compacts (holding `compact.lock`), another that reaches the threshold skips its compaction rather
    /// than writing the same merged pack at the same time; it compacts once the lock is free.
    #[test]
    fn a_store_skips_compacting_while_another_process_compacts() {
        let dir = temp_dir("lock");
        let (abi, _) = abi_info();
        let store = Store::open_at(dir.clone(), *abi);
        store.put(stress_key(0), Arc::from(stress_payload(0)));
        store.flush();
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("packs").join(COMPACT_LOCK)).unwrap();
        lock.try_lock().unwrap();
        for index in 1..=COMPACT_THRESHOLD + 2 {
            store.put(stress_key(index), Arc::from(stress_payload(index)));
            store.flush();
        }
        assert_eq!(store.packs.read().unwrap().len(), COMPACT_THRESHOLD + 3, "no compaction while another process holds the lock");
        drop(lock);
        store.put(stress_key(100), Arc::from(stress_payload(100)));
        store.flush();
        assert_eq!(store.packs.read().unwrap().len(), 1, "compacted once the lock was free");
        for index in (0..=COMPACT_THRESHOLD + 2).chain([100]) {
            assert_eq!(store.get(&stress_key(index)).as_deref(), Some(stress_payload(index).as_slice()), "entry {index}");
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

    #[test]
    fn deferred_flush_buffers_small_deltas_until_threshold_or_explicit_flush() {
        let dir = temp_dir("deferred");
        let (abi, _) = abi_info();
        let store = Store::open_at(dir.clone(), *abi);
        // Open sibling BEFORE `store` publishes anything so sibling's `put` genuinely buffers all
        // overlapping keys in memory and exercises the flush-time refresh + deduplication path.
        let sibling = Store::open_at(dir.clone(), *abi);
        for index in 0..4 {
            store.put(stress_key(index), Arc::from(format!("small-{index}").into_bytes()));
            store.flush_deferred();
        }
        assert!(pack_files(&dir).is_empty(), "small deltas stay buffered in memory");
        for index in 0..4 {
            assert_eq!(store.get(&stress_key(index)).as_deref(), Some(format!("small-{index}").as_bytes()));
        }
        // Populate both stores past DEFERRED_FLUSH_ENTRIES while the directory is still empty.
        for index in 4..DEFERRED_FLUSH_ENTRIES {
            store.put(stress_key(index), Arc::from(format!("small-{index}").into_bytes()));
        }
        for index in 0..DEFERRED_FLUSH_ENTRIES {
            sibling.put(stress_key(index), Arc::from(format!("small-{index}").into_bytes()));
        }
        sibling.put(stress_key(888), Arc::from(b"tiny-unique".as_slice()));
        assert_eq!(sibling.pending_entries.load(Ordering::Relaxed), DEFERRED_FLUSH_ENTRIES + 1);

        store.flush_deferred();
        assert_eq!(pack_files(&dir).len(), 1, "one pack published once threshold is reached");

        // Sibling enters flush_inner past threshold, refreshes from disk, drops the already-published
        // duplicates via map.retain, and keeps only its 1 unique entry buffered below threshold.
        sibling.flush_deferred();
        assert_eq!(pack_files(&dir).len(), 1, "post-dedup remainder below threshold stays buffered");
        assert_eq!(sibling.pending_entries.load(Ordering::Relaxed), 1, "only the unique entry should remain pending after dedup");
        sibling.flush();
        assert_eq!(pack_files(&dir).len(), 2);

        // Another small delta stays buffered until explicit flush.
        store.put(stress_key(999), Arc::from(b"tail".as_slice()));
        store.flush_deferred();
        assert_eq!(pack_files(&dir).len(), 2);
        store.flush();
        assert_eq!(pack_files(&dir).len(), 3);
        let reader = Store::open_at(dir.clone(), *abi);
        assert_eq!(reader.get(&stress_key(888)).as_deref(), Some(b"tiny-unique".as_slice()));
        assert_eq!(reader.get(&stress_key(999)).as_deref(), Some(b"tail".as_slice()));
        assert_eq!(reader.get(&stress_key(0)).as_deref(), Some(b"small-0".as_slice()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deferred_flush_skips_while_another_process_holds_flush_lock() {
        let dir = temp_dir("flush-lock-defer");
        let (abi, _) = abi_info();
        let store = Store::open_at(dir.clone(), *abi);
        let packs_dir = dir.join("packs");
        std::fs::create_dir_all(&packs_dir).unwrap();
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(packs_dir.join(FLUSH_LOCK)).unwrap();
        lock.try_lock().unwrap();
        for index in 0..DEFERRED_FLUSH_ENTRIES {
            store.put(stress_key(index), Arc::from(format!("locked-{index}").into_bytes()));
        }
        // Must return immediately without blocking or publishing while flush.lock is held.
        store.flush_deferred();
        assert!(pack_files(&dir).is_empty(), "deferred flush must skip while flush.lock is held");
        assert_eq!(store.pending_entries.load(Ordering::Relaxed), DEFERRED_FLUSH_ENTRIES);
        drop(lock);
        store.flush_deferred();
        assert_eq!(pack_files(&dir).len(), 1, "deferred flush publishes once flush.lock is free");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn miss_refresh_picks_up_sibling_pack_after_interval() {
        let dir = temp_dir("miss-refresh");
        let (abi, _) = abi_info();
        let writer = Store::open_at(dir.clone(), *abi);
        let reader = Store::open_at(dir.clone(), *abi);
        let key = stress_key(42);
        let payload = stress_payload(42);
        writer.put(key, Arc::from(payload.as_slice()));
        writer.flush();
        // Immediately after open (within MISS_REFRESH_INTERVAL_MS), miss probe is throttled.
        assert!(reader.get(&key).is_none());
        // Once the interval passes, a cold miss refreshes from disk and finds the sibling's pack.
        std::thread::sleep(Duration::from_millis(MISS_REFRESH_INTERVAL_MS + 60));
        assert_eq!(reader.get(&key).as_deref(), Some(payload.as_slice()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compaction_preserves_packs_outside_its_snapshot_and_races_cleanly_with_miss_refresh() {
        let dir = temp_dir("compact-snapshot-race");
        let (abi, _) = abi_info();
        let store = Store::open_at(dir.clone(), *abi);
        // Seed COMPACT_THRESHOLD packs and take a snapshot BEFORE an extra pack is loaded.
        for index in 0..COMPACT_THRESHOLD {
            store.put(stress_key(index), Arc::from(stress_payload(index)));
            store.flush();
        }
        let snapshot = store.packs.read().unwrap().clone();
        // Simulate a pack loaded into `store.packs` after `snapshot` was taken.
        let extra_path = write_stress_pack(&dir, 900..901);
        assert!(store.refresh_from_disk_locked());
        assert_eq!(store.get(&stress_key(900)).as_deref(), Some(stress_payload(900).as_slice()));
        // Compacting `snapshot` must keep `extra_path` in `store.packs` and never drop it while
        // keeping its name in `loaded`.
        store.compact_snapshot(snapshot);
        {
            let packs = store.packs.read().unwrap();
            let paths: FxHashSet<PathBuf> = packs.iter().map(|p| p.path.clone()).collect();
            assert_eq!(paths.len(), packs.len(), "pack list must never contain duplicate paths");
            assert!(paths.contains(&extra_path), "pack loaded outside snapshot must survive compaction");
        }
        assert_eq!(store.get(&stress_key(900)).as_deref(), Some(stress_payload(900).as_slice()));
        for index in 0..COMPACT_THRESHOLD {
            assert_eq!(store.get(&stress_key(index)).as_deref(), Some(stress_payload(index).as_slice()));
        }

        // Also hammer concurrent compactions + miss-refreshes across threads on one shared Store.
        let shared = Arc::clone(&store);
        let dir_writer = dir.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_reader = Arc::clone(&stop);
        let reader_thread = std::thread::spawn(move || {
            for step in 0..200 {
                if stop_reader.load(Ordering::Relaxed) {
                    break;
                }
                // Force miss-refresh eligibility on every iteration to maximize lock contention.
                shared.last_miss_probe_ms.store(0, Ordering::Relaxed);
                let _ = shared.get(&stress_key(50_000 + step));
                let packs = shared.packs.read().unwrap();
                let unique: FxHashSet<PathBuf> = packs.iter().map(|p| p.path.clone()).collect();
                assert_eq!(unique.len(), packs.len(), "concurrent miss-refresh + compaction duplicated a pack");
            }
        });
        for round in 0..4 {
            let base = 2_000 + round * (COMPACT_THRESHOLD + 2);
            for offset in 0..=COMPACT_THRESHOLD {
                write_stress_pack(&dir_writer, (base + offset)..(base + offset + 1));
                store.put(stress_key(10_000 + base + offset), Arc::from(stress_payload(10_000 + base + offset)));
                store.flush();
            }
        }
        stop.store(true, Ordering::Relaxed);
        reader_thread.join().unwrap();
        let verify = Store::open_at(dir.clone(), *abi);
        assert_eq!(verify.get(&stress_key(900)).as_deref(), Some(stress_payload(900).as_slice()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn killed_process_holding_flush_lock_releases_lock_for_next_flush() {
        if let Some(lock_path) = std::env::var_os("REE_MODSTORE_HOLD_LOCK") {
            let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path).unwrap();
            file.lock().unwrap();
            if let Some(ready_path) = std::env::var_os("REE_MODSTORE_HOLD_READY") {
                let _ = std::fs::write(&ready_path, b"locked");
            }
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        let dir = temp_dir("kill-lock");
        let (abi, _) = abi_info();
        let packs_dir = dir.join("packs");
        std::fs::create_dir_all(&packs_dir).unwrap();
        let lock_path = packs_dir.join(FLUSH_LOCK);
        let ready_path = dir.join("locked.ready");
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(&exe)
            .args(["lua::modules::store::tests::killed_process_holding_flush_lock_releases_lock_for_next_flush", "--exact", "--nocapture", "--test-threads=1"])
            .env("REE_MODSTORE_HOLD_LOCK", &lock_path)
            .env("REE_MODSTORE_HOLD_READY", &ready_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready_path.is_file() {
            assert!(Instant::now() < deadline, "lock-holder child did not acquire flush.lock in time");
            if let Ok(Some(status)) = child.try_wait() {
                panic!("lock-holder child exited early with {status}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let store = Store::open_at(dir.clone(), *abi);
        for index in 0..DEFERRED_FLUSH_ENTRIES {
            store.put(stress_key(index), Arc::from(format!("after-kill-{index}").into_bytes()));
        }
        // Deferred flush skips while the child holds flush.lock.
        store.flush_deferred();
        assert!(pack_files(&dir).is_empty(), "deferred flush must skip while child holds flush.lock");
        // Kill the holder abruptly; the OS must release flush.lock so the next flush succeeds.
        child.kill().unwrap();
        let _ = child.wait();
        store.flush();
        assert_eq!(pack_files(&dir).len(), 1, "flush must succeed after killed holder releases flush.lock");
        let reader = Store::open_at(dir.clone(), *abi);
        assert_eq!(reader.get(&stress_key(0)).as_deref(), Some(b"after-kill-0".as_slice()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frozen_mode_still_runs_deferred_flush_unless_in_process_batch_suppresses_it() {
        if let Some(dir) = std::env::var_os("REE_MODSTORE_FROZEN_CHILD") {
            let dir = PathBuf::from(dir);
            let store = for_state_dir(&dir).unwrap();
            for index in 0..DEFERRED_FLUSH_ENTRIES {
                store.put(stress_key(index), Arc::from(format!("frozen-{index}").into_bytes()));
            }
            // Frozen mode + deferred flush (rur engine-batch --frozen) MUST still run deferred flush.
            super::super::set_batch_frozen(true);
            set_deferred_flush(true);
            set_suppress_per_run_flush(false);
            flush_all();
            assert!(!store.has_pending.load(Ordering::Acquire), "frozen mode with deferred flush must publish past threshold");
            // In-process batch suppression (`ree run --batch`) skips per-run flush_all().
            for index in DEFERRED_FLUSH_ENTRIES..(DEFERRED_FLUSH_ENTRIES * 2) {
                store.put(stress_key(index), Arc::from(format!("suppressed-{index}").into_bytes()));
            }
            set_suppress_per_run_flush(true);
            flush_all();
            assert!(store.has_pending.load(Ordering::Acquire), "SUPPRESS_PER_RUN_FLUSH must defer until explicit flush_all_now");
            set_suppress_per_run_flush(false);
            flush_all_now();
            assert!(!store.has_pending.load(Ordering::Acquire));
            return;
        }
        let dir = temp_dir("frozen-deferred");
        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(&exe)
            .args(["lua::modules::store::tests::frozen_mode_still_runs_deferred_flush_unless_in_process_batch_suppresses_it", "--exact", "--nocapture", "--test-threads=1"])
            .env("REE_MODSTORE_FROZEN_CHILD", &dir)
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            output.status.success(),
            "frozen-deferred child failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn concurrent_flushes_deduplicate_overlapping_entries_on_disk() {
        let dir = temp_dir("flush-dedup");
        let (abi, _) = abi_info();
        let shared_payload: Arc<[u8]> = Arc::from(vec![0x5au8; 8192]);
        let workers = 8usize;
        let shared_count = 32usize;
        let barrier = Arc::new(std::sync::Barrier::new(workers));
        let mut handles = Vec::with_capacity(workers);
        for w in 0..workers {
            let dir = dir.clone();
            let abi = *abi;
            let shared_payload = shared_payload.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                let store = Store::open_at(dir, abi);
                for idx in 0..shared_count {
                    store.put(stress_key(idx), shared_payload.clone());
                }
                store.put(stress_key(10_000 + w), Arc::from(format!("unique-{w}").into_bytes()));
                barrier.wait();
                store.flush();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        let total_pack_bytes: u64 = pack_files(&dir)
            .iter()
            .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();
        let unique_bytes = (shared_count * 8192) as u64;
        assert!(
            total_pack_bytes < unique_bytes + 65_536,
            "overlapping concurrent flushes duplicated entries: total_pack_bytes={total_pack_bytes} unique_bytes={unique_bytes}"
        );
        let reader = Store::open_at(dir.clone(), *abi);
        for idx in 0..shared_count {
            assert_eq!(reader.get(&stress_key(idx)).as_deref(), Some(shared_payload.as_ref()));
        }
        for w in 0..workers {
            assert_eq!(reader.get(&stress_key(10_000 + w)).as_deref(), Some(format!("unique-{w}").as_bytes()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
