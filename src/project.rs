//! Builds the virtual DataModel (port of `runtime/engine/project.luau`): Studio snapshots and
//! models (.rbxm / .rbxmx through rbx-dom) and filesystem script trees mounted over them (Rojo
//! layout `<root>/<Service>/...`: init.luau, *.server.luau, *.client.luau, init.meta.json,
//! JSON modules, *.model.json models), offline stand-ins for Studio-only packages, and Studio-only
//! placeholders from the place's hierarchy snapshot, created when a script first looks up or lists
//! children.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use rbx_dom_weak::WeakDom;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    ClassId, Engine, InstanceId, K, NameId, Reflection, State,
    dom::{StudioNode, flags},
    instance as core, services,
    value::{Str, Value},
};

pub mod json_model;
#[cfg(feature = "bundles")]
pub mod model_assets;
pub mod plan;
pub mod received;

pub use plan::{LazyImport, ModelPlan, PlanProps, PlanPropsIter, Unmodeled, before_method, convert_instances, find_unmade, materialize_lazy, materialize_lazy_subtree, materialize_region};

/// A run's own choice of asset manifest bundles to mount, over rur.json `modelAssets` (`rur-native
/// record --assets-at <Service.Path>` / `--assets <prefix>,...`): where they mount (a playing client
/// finds its delivered bundles elsewhere than an edit-mode story) and which (`*`: every bundle).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelAssetsOverride {
    pub mount: Option<String>,
    pub prefixes: Option<Vec<String>>,
    /// What the run is (`ree record` / `ree run`: its harness, place and context): the bundles decoded
    /// ahead are the ones the last run of the same scene imported, not the last run of the mount's
    pub scene: Option<String>,
}

/// Studio hierarchy data and diagnostics of the mounted place.
#[derive(Default)]
pub struct ProjectState {
    #[cfg(feature = "bundles")]
    pub model_assets: model_assets::ModelAssets,
    /// The running client's PlayerGui created in edit mode (init.luau MissingChild)
    pub client_gui: Option<InstanceId>,
    /// Engine.ClientGuiMount (main.luau, rur.json clientGui): mounts what a play-testing client
    /// builds in its PlayerGui (story.rs sets it; lua/globals.rs missing_child runs it once)
    pub client_gui_mount: Option<mlua::Function>,
    /// The PlayerGui was created while a client gui mount was set, and it has not run yet
    pub client_mount_pending: bool,
    studio: Option<Studio>,
    stand_in_sources: FxHashMap<String, Rc<[u8]>>,
    used_placeholders: FxHashSet<InstanceId>,
    pub studio_only_count: u32,
    pub studio_only_used: Vec<String>,
    pub studio_only_gui: Vec<String>,
    pub hierarchy_path: Option<String>,
    /// The Folders the mount made for directories that name no class of their own (no init script, no init.meta.json
    /// className): [`use_studio_hierarchy`] gives each the class the place's hierarchy has at its path
    implicit_dirs: Vec<InstanceId>,
    /// Mounted files' stamps from the project's listing ([`ScanNode::stamps`])
    pub stamps: FxHashMap<String, crate::files::Stamp>,
    /// Plans imported lazily ([`ModelPlan::instantiate_lazy`])
    pub imports: Vec<LazyImport>,
    /// The lazily imported instances whose plan children are not made yet: (import, node)
    pub lazy: FxHashMap<InstanceId, (u32, u32)>,
    /// Instances made of lazy imports so far
    pub lazy_made: u32,
    /// While a lazy import makes everything below instances it made in a world (plan.rs make_level)
    pub making_whole: bool,
    /// The directory listings the mount made, kept when a prepared run will check them again
    /// (record/prepare.rs: Some before the mount)
    pub listings: Option<Vec<Listing>>,
}

/// A directory tree as a mount listed it: what a later [`Listing::unchanged`] compares the directory
/// with now.
pub struct Listing(ScanNode);

impl Listing {
    /// Whether the directory listed now is as it was: the same entries, of the same kinds, in the same
    /// order, and the same stamps for every file but the Luau sources not in `read` (a source the run
    /// did not read may change: the run reads it later, as it is then).
    pub fn unchanged(&self, read: &FxHashSet<String>) -> bool {
        same_listing(&self.0, &scan(self.0.path.clone()), read)
    }

    pub fn path(&self) -> &str {
        &self.0.path
    }
}

fn same_listing(old: &ScanNode, new: &ScanNode, read: &FxHashSet<String>) -> bool {
    if old.entries != new.entries || old.stamps.len() != new.stamps.len() || old.dirs.len() != new.dirs.len() || old.loops != new.loops {
        return false;
    }
    for (index, entry) in old.entries.iter().enumerate() {
        let path = format!("{}/{entry}", old.path);
        match (old.dirs.get(entry), new.dirs.get(entry)) {
            (Some(old_dir), Some(new_dir)) => {
                if !same_listing(old_dir, new_dir, read) {
                    return false;
                }
            }
            // a link left out still leads where it did (a link has no stamp of its own)
            (None, None) if old.loops.iter().any(|(name, _)| name == entry) => {}
            (None, None) => {
                let (before, now) = (old.stamps.get(index).copied().flatten(), new.stamps.get(index).copied().flatten());
                let source = ends_with_ignore_ascii_case(entry, ".luau") || ends_with_ignore_ascii_case(entry, ".lua");
                if (!source || read.contains(&path)) && (before.is_none() || before != now) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

#[inline]
fn ends_with_ignore_ascii_case(text: &str, suffix: &str) -> bool {
    text.len() >= suffix.len() && text.as_bytes()[text.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

#[inline]
fn starts_with_ignore_ascii_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Keeps `node` when the mount keeps its listings ([`ProjectState::listings`]).
fn keep_listing(engine: &Engine, node: ScanNode) {
    if let Some(listings) = engine.state.borrow_mut().project.listings.as_mut() {
        listings.push(Listing(node));
    }
}

struct Studio {
    game: InstanceId,
    index: std::sync::Arc<StudioIndex>,
}

const STUDIO_INDEX_MAGIC: &[u8; 8] = b"REESTUD1";
const STUDIO_INDEX_TRAILER: &[u8; 8] = b"ENDSTUD1";
const STUDIO_INDEX_VERSION: u32 = 2;
const STUDIO_INDEX_HEADER_BYTES: usize = 48;
const STUDIO_INDEX_MAX_FILES: usize = 16;

enum StudioBytes {
    Mapped(std::sync::Arc<plan::Mapping>),
    /// 4-byte-aligned owned backing for non-mmap platforms or fallback.
    Owned { words: Box<[u32]>, byte_len: usize },
}

impl StudioBytes {
    fn as_bytes(&self) -> &[u8] {
        match self {
            StudioBytes::Mapped(mapping) => mapping.bytes(),
            StudioBytes::Owned { words, byte_len } => unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), *byte_len) },
        }
    }

    /// Reads `count` little-endian `u32`s starting at 4-byte-aligned byte `offset`.
    #[inline]
    fn u32_slice(&self, offset: usize, count: usize) -> &[u32] {
        let bytes = self.as_bytes();
        debug_assert!(offset % 4 == 0 && offset + count * 4 <= bytes.len());
        #[cfg(target_endian = "little")]
        unsafe {
            std::slice::from_raw_parts(bytes.as_ptr().add(offset).cast::<u32>(), count)
        }
        #[cfg(not(target_endian = "little"))]
        compile_error!("StudioIndex binary layout assumes little-endian")
    }
}

/// A place's Studio hierarchy as the mount reads it: compact 4-byte-aligned CSR + interned class
/// table, mmapped read-only from `<state_dir>/studio/v1-<hash>.studio` so processes share pages
/// without parsing JSON or allocating tens of thousands of per-node Strings/HashMaps.
pub struct StudioIndex {
    bytes: StudioBytes,
    node_count: u32,
    class_count: u32,
    name_bytes_off: usize,
    name_bytes_len: usize,
    name_ends_off: usize,
    class_bytes_off: usize,
    class_bytes_len: usize,
    class_ends_off: usize,
    node_classes_off: usize,
    parents_off: usize,
    child_offsets_off: usize,
    child_nodes_off: usize,
    child_nodes_len: usize,
    stand_ins_off: usize,
    stand_in_count: usize,
}

impl StudioIndex {
    fn align4(value: usize) -> usize {
        (value + 3) & !3
    }

    fn encode(snapshot: &HierarchySnapshot) -> Vec<u8> {
        let nodes = &snapshot.nodes;
        let node_count = nodes.len();
        let mut name_bytes = Vec::with_capacity(node_count * 12);
        let mut name_ends: Vec<u32> = Vec::with_capacity(node_count);
        let mut class_map: FxHashMap<&str, u32> = FxHashMap::default();
        let mut class_bytes = Vec::new();
        let mut class_ends: Vec<u32> = Vec::new();
        let mut node_classes: Vec<u32> = Vec::with_capacity(node_count);
        let mut parents: Vec<u32> = Vec::with_capacity(node_count);
        let mut child_counts: Vec<u32> = vec![0; node_count];
        let mut stand_ins: Vec<u32> = Vec::new();

        for (index, node) in nodes.iter().enumerate() {
            name_bytes.extend_from_slice(node.name.as_bytes());
            name_ends.push(name_bytes.len() as u32);
            let class_idx = match class_map.get(node.class_name.as_str()) {
                Some(&id) => id,
                None => {
                    let id = class_ends.len() as u32;
                    class_bytes.extend_from_slice(node.class_name.as_bytes());
                    class_ends.push(class_bytes.len() as u32);
                    class_map.insert(node.class_name.as_str(), id);
                    id
                }
            };
            node_classes.push(class_idx);
            let parent_1based = match node.parent {
                Some(p) if p >= 0 && (p as usize) < node_count => {
                    child_counts[p as usize] += 1;
                    p as u32 + 1
                }
                _ => 0,
            };
            parents.push(parent_1based);
            if node.class_name == "ModuleScript" && stand_in_file(&node.name).is_some() {
                stand_ins.push(index as u32 + 1);
            }
        }

        let mut child_offsets: Vec<u32> = Vec::with_capacity(node_count + 1);
        let mut cursor = 0u32;
        for &count in &child_counts {
            child_offsets.push(cursor);
            cursor += count;
        }
        child_offsets.push(cursor);
        let mut write_pos = child_offsets[..node_count].to_vec();
        let mut child_nodes: Vec<u32> = vec![0; cursor as usize];
        for (index, &parent_1based) in parents.iter().enumerate() {
            if parent_1based != 0 {
                let slot = &mut write_pos[parent_1based as usize - 1];
                child_nodes[*slot as usize] = index as u32 + 1;
                *slot += 1;
            }
        }

        let name_padded = Self::align4(name_bytes.len());
        let class_padded = Self::align4(class_bytes.len());
        let total = STUDIO_INDEX_HEADER_BYTES
            + name_padded
            + node_count * 4
            + class_padded
            + class_ends.len() * 4
            + node_count * 4
            + node_count * 4
            + (node_count + 1) * 4
            + child_nodes.len() * 4
            + stand_ins.len() * 4
            + STUDIO_INDEX_TRAILER.len();

        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(STUDIO_INDEX_MAGIC);
        out.extend_from_slice(&STUDIO_INDEX_VERSION.to_le_bytes());
        out.extend_from_slice(&(node_count as u32).to_le_bytes());
        out.extend_from_slice(&(class_ends.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(class_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(child_nodes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(stand_ins.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let checksum_off = out.len();
        out.extend_from_slice(&0u64.to_le_bytes());

        out.extend_from_slice(&name_bytes);
        out.resize(out.len() + (name_padded - name_bytes.len()), 0);
        for &v in &name_ends {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&class_bytes);
        out.resize(out.len() + (class_padded - class_bytes.len()), 0);
        for &v in &class_ends {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for &v in &node_classes {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for &v in &parents {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for &v in &child_offsets {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for &v in &child_nodes {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for &v in &stand_ins {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(STUDIO_INDEX_TRAILER);
        debug_assert_eq!(out.len(), total);
        let checksum = hash_payload_64(&out[STUDIO_INDEX_HEADER_BYTES..]);
        out[checksum_off..checksum_off + 8].copy_from_slice(&checksum.to_le_bytes());
        out
    }

    fn from_bytes(bytes: StudioBytes) -> Option<StudioIndex> {
        let raw = bytes.as_bytes();
        if raw.len() < STUDIO_INDEX_HEADER_BYTES + STUDIO_INDEX_TRAILER.len() || raw.len() % 4 != 0 {
            return None;
        }
        if &raw[..8] != STUDIO_INDEX_MAGIC || &raw[raw.len() - 8..] != STUDIO_INDEX_TRAILER {
            return None;
        }
        let u32_at = |off: usize| -> u32 { u32::from_le_bytes(raw[off..off + 4].try_into().unwrap()) };
        let version = u32_at(8);
        if version != STUDIO_INDEX_VERSION {
            return None;
        }
        let node_count = u32_at(12);
        let class_count = u32_at(16);
        let name_bytes_len = u32_at(20) as usize;
        let class_bytes_len = u32_at(24) as usize;
        let child_nodes_len = u32_at(28) as usize;
        let stand_in_count = u32_at(32) as usize;
        let reserved = u32_at(36);
        if reserved != 0 {
            return None;
        }
        let stored_checksum = u64::from_le_bytes(raw[40..48].try_into().unwrap());
        if stored_checksum != hash_payload_64(&raw[STUDIO_INDEX_HEADER_BYTES..]) {
            return None;
        }

        let n = node_count as usize;
        let c = class_count as usize;
        let mut off = STUDIO_INDEX_HEADER_BYTES;
        let name_bytes_off = off;
        off = off.checked_add(Self::align4(name_bytes_len))?;
        let name_ends_off = off;
        off = off.checked_add(n.checked_mul(4)?)?;
        let class_bytes_off = off;
        off = off.checked_add(Self::align4(class_bytes_len))?;
        let class_ends_off = off;
        off = off.checked_add(c.checked_mul(4)?)?;
        let node_classes_off = off;
        off = off.checked_add(n.checked_mul(4)?)?;
        let parents_off = off;
        off = off.checked_add(n.checked_mul(4)?)?;
        let child_offsets_off = off;
        off = off.checked_add(n.checked_add(1)?.checked_mul(4)?)?;
        let child_nodes_off = off;
        off = off.checked_add(child_nodes_len.checked_mul(4)?)?;
        let stand_ins_off = off;
        off = off.checked_add(stand_in_count.checked_mul(4)?)?;
        let expected_len = off.checked_add(STUDIO_INDEX_TRAILER.len())?;
        if raw.len() != expected_len {
            return None;
        }

        // Validate table invariants once on open so every lookup is bounds- and UTF-8-safe.
        let class_text = std::str::from_utf8(&raw[class_bytes_off..class_bytes_off + class_bytes_len]).ok()?;
        let class_ends = bytes.u32_slice(class_ends_off, c);
        let mut prev = 0u32;
        for &end in class_ends {
            if end < prev || (end as usize) > class_bytes_len || !class_text.is_char_boundary(end as usize) {
                return None;
            }
            prev = end;
        }

        let name_ends = bytes.u32_slice(name_ends_off, n);
        let mut prev = 0u32;
        for &end in name_ends {
            if end < prev || (end as usize) > name_bytes_len {
                return None;
            }
            prev = end;
        }

        if n > 0 {
            if c == 0 {
                return None;
            }
            for &cls in bytes.u32_slice(node_classes_off, n) {
                if cls >= class_count {
                    return None;
                }
            }
        }
        for &parent in bytes.u32_slice(parents_off, n) {
            if parent > node_count {
                return None;
            }
        }
        let child_offsets = bytes.u32_slice(child_offsets_off, n + 1);
        if child_offsets[0] != 0 || child_offsets[n] as usize != child_nodes_len {
            return None;
        }
        let mut prev = 0u32;
        for &co in child_offsets {
            if co < prev {
                return None;
            }
            prev = co;
        }
        for &child in bytes.u32_slice(child_nodes_off, child_nodes_len) {
            if child == 0 || child > node_count {
                return None;
            }
        }
        for &si in bytes.u32_slice(stand_ins_off, stand_in_count) {
            if si == 0 || si > node_count {
                return None;
            }
        }

        Some(StudioIndex {
            bytes,
            node_count,
            class_count,
            name_bytes_off,
            name_bytes_len,
            name_ends_off,
            class_bytes_off,
            class_bytes_len,
            class_ends_off,
            node_classes_off,
            parents_off,
            child_offsets_off,
            child_nodes_off,
            child_nodes_len,
            stand_ins_off,
            stand_in_count,
        })
    }

    fn from_owned(encoded: Vec<u8>) -> Option<StudioIndex> {
        if encoded.len() % 4 != 0 {
            return None;
        }
        let word_count = encoded.len() / 4;
        let mut words: Vec<u32> = Vec::with_capacity(word_count);
        unsafe {
            std::ptr::copy_nonoverlapping(encoded.as_ptr(), words.as_mut_ptr().cast::<u8>(), encoded.len());
            words.set_len(word_count);
        }
        Self::from_bytes(StudioBytes::Owned { words: words.into_boxed_slice(), byte_len: encoded.len() })
    }

    fn open_mapped(path: &Path) -> Option<StudioIndex> {
        if let Some(mapping) = plan::Mapping::open(path) {
            return Self::from_bytes(StudioBytes::Mapped(mapping));
        }
        // Fallback when mmap is unavailable (e.g., non-Windows)
        let encoded = std::fs::read(path).ok()?;
        Self::from_owned(encoded)
    }

    #[inline]
    fn node_count(&self) -> usize {
        self.node_count as usize
    }

    /// Node `node`'s name (1-based).
    #[inline]
    fn name(&self, node: u32) -> &[u8] {
        let idx = node as usize - 1;
        let ends = self.bytes.u32_slice(self.name_ends_off, self.node_count as usize);
        let end = ends[idx] as usize;
        let start = if idx > 0 { ends[idx - 1] as usize } else { 0 };
        &self.bytes.as_bytes()[self.name_bytes_off..self.name_bytes_off + self.name_bytes_len][start..end]
    }

    /// Node `node`'s class name (1-based).
    #[inline]
    fn class_name(&self, node: u32) -> &str {
        let cls = self.bytes.u32_slice(self.node_classes_off, self.node_count as usize)[node as usize - 1] as usize;
        let ends = self.bytes.u32_slice(self.class_ends_off, self.class_count as usize);
        let end = ends[cls] as usize;
        let start = if cls > 0 { ends[cls - 1] as usize } else { 0 };
        let slice = &self.bytes.as_bytes()[self.class_bytes_off..self.class_bytes_off + self.class_bytes_len][start..end];
        // SAFETY: validated UTF-8 and char boundaries in `from_bytes`.
        unsafe { std::str::from_utf8_unchecked(slice) }
    }

    /// Node `node`'s 1-based parent (`0` if root / no parent).
    #[inline]
    fn parent(&self, node: u32) -> u32 {
        self.bytes.u32_slice(self.parents_off, self.node_count as usize)[node as usize - 1]
    }

    /// Children (1-based node indices) of 1-based `node`.
    #[inline]
    fn children(&self, node: u32) -> &[u32] {
        if node == 0 || node > self.node_count {
            return &[];
        }
        let offsets = self.bytes.u32_slice(self.child_offsets_off, self.node_count as usize + 1);
        let start = offsets[node as usize - 1] as usize;
        let end = offsets[node as usize] as usize;
        &self.bytes.u32_slice(self.child_nodes_off, self.child_nodes_len)[start..end]
    }

    /// Pre-indexed 1-based node ids of stand-in ModuleScript packages.
    #[inline]
    fn stand_ins(&self) -> &[u32] {
        self.bytes.u32_slice(self.stand_ins_off, self.stand_in_count)
    }
}

const SCRIPT_SUFFIXES: &[(&str, &str)] = &[
    (".server.luau", "Script"),
    (".server.lua", "Script"),
    (".client.luau", "LocalScript"),
    (".client.lua", "LocalScript"),
    (".luau", "ModuleScript"),
    (".lua", "ModuleScript"),
];

/// Offline stand-ins (lua/packages/*, compiled in) for packages a place keeps only in Studio.
const PACKAGE_STAND_INS: &[(&str, &str)] = &[("ui-labs", "ui-labs.luau"), ("UILabs", "ui-labs.luau")];

fn stand_in_file(name: &str) -> Option<&'static str> {
    PACKAGE_STAND_INS.iter().find(|(n, _)| *n == name).map(|(_, f)| *f)
}

#[derive(Default, Debug, Clone, Copy)]
pub struct MountCounts {
    pub created: u32,
    pub scripts: u32,
    pub conflicts: u32,
    pub skipped: u32,
    /// The projects being mounted on the way down to the directory at hand
    project_depth: u32,
    /// The projects this mount has followed
    projects: u32,
}

fn is_a(reflection: &Reflection, class: ClassId, ancestor: &str) -> bool {
    reflection.is_a(class, ancestor)
}

fn find_child(state: &State, parent: InstanceId, name: &str) -> Option<InstanceId> {
    state.dom.find_child(parent, name.as_bytes())
}

// Studio-only instances

/// The node of the Studio place's index that `id` stands for, by its name under its parent's node: the place's game is node 1,
/// a script's instance and one with no parent none. Kept on each instance worked out. Walked up to the nearest instance whose
/// node is known or settled, then back down, not recursed: a script can nest clones of a place's instances deeper than any
/// thread's stack holds frames for.
fn studio_node_of(state: &mut State, id: InstanceId) -> Option<u32> {
    let settle = |state: &mut State, id: InstanceId, node: Option<u32>| {
        state.dom.at_mut(id).studio_node = match node {
            Some(node) => StudioNode::Node(node),
            None => StudioNode::None,
        };
    };
    // (the instances whose node comes from their parent's, the nearest to `id` first)
    let mut below: Vec<InstanceId> = Vec::new();
    let mut at = id;
    let mut node = loop {
        match state.dom.at(at).studio_node {
            StudioNode::Node(node) => break Some(node),
            StudioNode::None => break None,
            StudioNode::Unknown => {}
        }
        let studio_game = state.project.studio.as_ref()?.game;
        let instance = state.dom.at(at);
        let found = if at == studio_game {
            Some(1)
        } else if let Some(parent) = instance.parent.filter(|_| !instance.has(flags::SCRIPT_CREATED)) {
            below.push(at);
            at = parent;
            continue;
        } else {
            None
        };
        settle(state, at, found);
        break found;
    };
    while let Some(child) = below.pop() {
        let name = state.dom.at(child).name.clone();
        let studio = &state.project.studio.as_ref()?.index;
        node = node.and_then(|node| studio.children(node).iter().copied().find(|&found| studio.name(found) == name.as_bytes()));
        settle(state, child, node);
    }
    node
}

/// Hooks.Materialize: adds the Studio-only children of `id` once (and makes a lazily imported
/// instance's children).
pub fn materialize(state: &mut State, reflection: &Reflection, id: InstanceId) {
    if state.dom.at(id).has(flags::LAZY_PENDING) {
        materialize_lazy(state, reflection, id);
    }
    if state.dom.at(id).has(flags::MATERIALIZED) {
        return;
    }
    state.dom.at_mut(id).set_flag(flags::MATERIALIZED, true);
    let Some((game, index)) = state.project.studio.as_ref().map(|s| (s.game, s.index.clone())) else { return };
    if id == game {
        return;
    }
    let Some(node) = studio_node_of(state, id) else { return };
    let children = index.children(node);
    if children.is_empty() {
        return;
    }
    // The k-th Studio child named N corresponds to the k-th existing child named N
    let mut existing: FxHashMap<Str, u32> = FxHashMap::default();
    for &child in &state.dom.at(id).children {
        *existing.entry(state.dom.at(child).name.clone()).or_default() += 1;
    }
    let mut seen: FxHashMap<Str, u32> = FxHashMap::default();
    for &child_node in children {
        let name = Str::new(index.name(child_node));
        let class_name = index.class_name(child_node);
        let count = seen.entry(name.clone()).or_default();
        *count += 1;
        if *count <= existing.get(&name).copied().unwrap_or(0) {
            continue;
        }
        // Scripts that only exist in Studio stay absent, except packages with stand-ins
        let class = reflection.class_id(class_name);
        let is_script = class.is_some_and(|c| is_a(reflection, c, "LuaSourceContainer"));
        let stand_in = stand_in_file(&name.to_str_lossy()).filter(|_| class_name == "ModuleScript");
        if is_script && stand_in.is_none() {
            continue;
        }
        let create_class = match class {
            Some(c) if !reflection.class(c).service => c,
            _ => reflection.class_id("Folder").expect("Folder"),
        };
        let placeholder = core::create(state, reflection, create_class);
        core::raw_set_silent(state, reflection, placeholder, K::Name as NameId, Value::String(name.clone()));
        let instance = state.dom.at_mut(placeholder);
        instance.studio_node = StudioNode::Node(child_node);
        instance.set_flag(flags::STUDIO_ONLY, true);
        if let Some(file) = stand_in {
            if let Some(source) = state.project.stand_in_sources.get(file).cloned() {
                let instance = state.dom.at_mut(placeholder);
                instance.set_source(Some(source));
                instance.set_stand_in(Some(Rc::from(file)));
            }
        }
        state.dom.attach(placeholder, id);
        state.project.studio_only_count += 1;
    }
}

/// A loopback client's place hierarchy: the server's snapshot (shared), its own DataModel at the top, and the same stand-in
/// sources (loopback/lazy.rs: each side makes the Studio-only instances of what it copied)
pub fn share_studio(server: &State, client: &mut State, client_game: InstanceId) {
    client.project.studio = server.project.studio.as_ref().map(|studio| Studio { game: client_game, index: studio.index.clone() });
    client.project.stand_in_sources = server.project.stand_in_sources.clone();
    client.project.hierarchy_path = server.project.hierarchy_path.clone();
}

/// Whether `id` waits to make instances only the server can (a lazily imported plan's, a bundle's: not the Studio
/// snapshot's, which a loopback client makes itself)
pub fn lazy_beyond_studio(state: &State, id: InstanceId) -> bool {
    if state.dom.at(id).flags & (flags::LAZY_PENDING | flags::BUNDLE_ROOT | flags::LAZY_ABOVE) != 0 {
        return true;
    }
    #[cfg(feature = "bundles")]
    if state.project.model_assets.waits_at(id) {
        return true;
    }
    false
}

/// MaterializeDeep: materializes a whole subtree (Clone, GetDescendants).
pub fn materialize_deep(engine: &Engine, id: InstanceId) {
    let mut stack = vec![id];
    while let Some(node) = stack.pop() {
        crate::lua::instance::materialize(engine, node);
        engine.state.borrow().dom.at(node).children.copy_to(&mut stack);
    }
}

/// Hooks.PlaceholderUsed: a script reached a Studio-only placeholder by name (diagnostics).
pub fn placeholder_used(state: &mut State, reflection: &Reflection, id: InstanceId) {
    if !state.project.used_placeholders.insert(id) {
        return;
    }
    let class = state.dom.at(id).class;
    let full_name = state.dom.full_name(reflection, id);
    let class_name = reflection.class(class).name.to_string();
    if is_a(reflection, class, "GuiBase2d") && state.project.studio_only_gui.len() < 40 {
        state.project.studio_only_gui.push(format!("{full_name} ({class_name})"));
    }
    if let Some(parent) = state.dom.at(id).parent {
        if state.project.used_placeholders.contains(&parent) {
            return;
        }
    }
    if state.project.studio_only_used.len() < 25 {
        state.project.studio_only_used.push(format!("{full_name} ({class_name})"));
    }
}

// Paths

fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

fn parent_path(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[..index].to_string(),
        None => String::new(),
    }
}

fn batch_read_json(path: &str) -> Option<serde_json::Value> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<FxHashMap<String, Option<serde_json::Value>>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| crate::alloc::long_lived(Default::default));
    if let Some(found) = map.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        return found.clone();
    }
    let parsed = std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let stored = crate::alloc::long_lived(|| parsed.clone());
    crate::alloc::long_lived(|| {
        map.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_string(), stored);
    });
    parsed
}

fn read_json(path: &str) -> Option<serde_json::Value> {
    crate::files::note(Path::new(path));
    if crate::lua::modules::is_batch_frozen() {
        return batch_read_json(path);
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// A project's places, at its folder: `{"environments": [{"name": "<Place>", "projectFile":
/// "<root>/default.project.json", "studioPlaceId": <id>}]}`. A root that is the folder of a place's
/// project file belongs to that place ([`find_hierarchy_snapshot`], [`declared_place_id`]).
pub const PLACES_FILE: &str = "studio-places.json";
/// Where a project keeps each place's Studio hierarchy snapshot, `<dir>/<Place>.json` (`{"nodes":
/// [{"name", "className", "parent"}]}`, a node's parent an index into the list, -1 for the DataModel)
pub const SNAPSHOTS_DIR: &str = "studio-snapshots";
/// A project's data roots: `{"<key>": ["<path>", ...]}` at its folder lists the folders that each
/// `<project>/external/<Name>/src` checkout holds for the game ([`data_roots`]); a path may start with
/// the prefix, which is dropped.
pub const DATA_ROOTS_FILE: &str = "data-roots.json";
pub const DATA_ROOTS_KEY: &str = "paths";
const DATA_ROOTS_PREFIX: &str = "";
/// The Studio names of mounted files whose names a file system cannot hold, at a project's folder
/// ([`apply_studio_names`])
pub const STUDIO_NAME_MAP: &str = "studio-name-map.json";

/// A place's Studio hierarchy snapshot (`<project>/<SNAPSHOTS_DIR>/<Place>.json`), parsed once
/// per process and file modification time.
#[derive(serde::Deserialize)]
pub struct HierarchySnapshot {
    pub nodes: Vec<HierarchyNode>,
}

#[derive(serde::Deserialize)]
pub struct HierarchyNode {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "className")]
    pub class_name: String,
    #[serde(default)]
    pub parent: Option<i64>,
}

pub fn load_hierarchy(path: &str) -> Option<std::sync::Arc<HierarchySnapshot>> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Entry = (String, Option<std::time::SystemTime>, Arc<HierarchySnapshot>);
    static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    crate::files::note(Path::new(path));
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(entries) = cache.lock() {
        if let Some((_, _, found)) = entries.iter().find(|(p, m, _)| p == path && *m == modified) {
            return Some(found.clone());
        }
    }
    let bytes = std::fs::read(path).ok()?;
    // (kept for the process: parsed into the long-lived heap)
    let snapshot: HierarchySnapshot = crate::alloc::long_lived(|| serde_json::from_slice(&bytes)).ok()?;
    let snapshot = crate::alloc::long_lived(|| Arc::new(snapshot));
    if let Ok(mut entries) = cache.lock() {
        entries.retain(|(p, _, _)| p != path);
        crate::alloc::long_lived(|| entries.push((path.to_string(), modified, snapshot.clone())));
    }
    Some(snapshot)
}

/// Fast streaming 128-bit content hash over 8-byte words (used for content-addressed StudioIndex files).
/// Partial trailing bytes are buffered across `update` calls so arbitrary read boundaries hash identically
/// to a single-shot buffer.
#[derive(Clone, Copy)]
struct Hash128 {
    a: u64,
    b: u64,
    len: u64,
    tail: [u8; 8],
    tail_len: u8,
}

impl Hash128 {
    fn new() -> Self {
        let mut h = Hash128 {
            a: 0x9e37_79b9_7f4a_7c15,
            b: 0xc2b2_ae3d_27d4_eb4f,
            len: 0,
            tail: [0u8; 8],
            tail_len: 0,
        };
        // Bind the stand-in package list into the content key so changing it invalidates old files.
        for &(name, file) in PACKAGE_STAND_INS {
            h.update(name.as_bytes());
            h.update(&[0]);
            h.update(file.as_bytes());
            h.update(&[0xff]);
        }
        h.flush_tail();
        h.len = 0;
        h
    }

    fn new_raw() -> Self {
        Hash128 {
            a: 0x517c_c1b7_2722_0a95,
            b: 0x94d0_49bb_1331_11eb,
            len: 0,
            tail: [0u8; 8],
            tail_len: 0,
        }
    }

    #[inline]
    fn mix_word(&mut self, word: u64) {
        self.a = (self.a ^ word).wrapping_mul(0x9e37_79b1_85eb_ca87).rotate_left(27);
        self.b = (self.b.wrapping_add(word)).wrapping_mul(0xc2b2_ae3d_27d4_eb4f).rotate_left(31);
    }

    fn flush_tail(&mut self) {
        let n = self.tail_len as usize;
        if n > 0 {
            for slot in &mut self.tail[n..] {
                *slot = 0;
            }
            let word = u64::from_le_bytes(self.tail);
            self.a = (self.a ^ word).wrapping_mul(0x9e37_79b1_85eb_ca87).rotate_left(27);
            self.b = (self.b.wrapping_add(word ^ (n as u64))).wrapping_mul(0xc2b2_ae3d_27d4_eb4f).rotate_left(31);
            self.tail = [0u8; 8];
            self.tail_len = 0;
        }
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.len = self.len.wrapping_add(bytes.len() as u64);
        if self.tail_len > 0 {
            let need = 8 - self.tail_len as usize;
            let take = need.min(bytes.len());
            self.tail[self.tail_len as usize..self.tail_len as usize + take].copy_from_slice(&bytes[..take]);
            self.tail_len += take as u8;
            bytes = &bytes[take..];
            if self.tail_len == 8 {
                let word = u64::from_le_bytes(self.tail);
                self.tail = [0u8; 8];
                self.tail_len = 0;
                self.mix_word(word);
            }
        }
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.mix_word(u64::from_le_bytes(chunk.try_into().unwrap()));
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            self.tail[..rem.len()].copy_from_slice(rem);
            self.tail_len = rem.len() as u8;
        }
    }

    fn finish(mut self) -> (u64, u64) {
        self.flush_tail();
        let mut a = self.a ^ self.len;
        let mut b = self.b.wrapping_add(self.len.rotate_left(17));
        a ^= a >> 33;
        a = a.wrapping_mul(0xff51_afd7_ed55_8ccd);
        a ^= a >> 33;
        a = a.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        a ^= a >> 33;
        b ^= b >> 30;
        b = b.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        b ^= b >> 27;
        b = b.wrapping_mul(0x94d0_49bb_1331_11eb);
        b ^= b >> 31;
        (a ^ b.rotate_left(13), b ^ a.rotate_right(19))
    }
}

pub(crate) fn hash_payload_64(bytes: &[u8]) -> u64 {
    let mut h = Hash128::new_raw();
    h.update(bytes);
    let (a, b) = h.finish();
    a ^ b.rotate_left(7)
}

fn hash_bytes_128(bytes: &[u8]) -> (u64, u64) {
    let mut h = Hash128::new();
    h.update(bytes);
    h.finish()
}

fn hash_file_128(path: &Path) -> Option<(u64, u64)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut h = Hash128::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finish())
}

/// Whether `stamp`'s mtime is recent enough relative to `now` that a same-length rewrite could share the same tick.
fn stamp_is_recent_at(stamp: crate::files::Stamp, now: std::time::SystemTime) -> bool {
    let Ok(since_epoch) = now.duration_since(std::time::UNIX_EPOCH) else {
        return true;
    };
    #[cfg(windows)]
    let (now, window) = ((since_epoch.as_nanos() / 100) as u64 + 116_444_736_000_000_000, 20_000_000u64);
    #[cfg(not(windows))]
    let (now, window) = (since_epoch.as_nanos() as u64, 2_000_000_000u64);
    stamp.modified.abs_diff(now) < window
}

/// Per-user cache root (`%LOCALAPPDATA%/ree`, `~/Library/Caches/ree`, `$XDG_CACHE_HOME/ree`, or
/// `~/.cache/ree`), matching `lib.rs::per_user_cache_dir`. Returns `None` if unavailable.
pub(crate) fn per_user_ree_cache_dir() -> Option<PathBuf> {
    let env_path = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let cache = match std::env::consts::OS {
        "windows" => env_path("LOCALAPPDATA"),
        "macos" => env_path("HOME").map(|home| home.join("Library/Caches")),
        _ => env_path("XDG_CACHE_HOME").filter(|dir| dir.is_absolute()).or_else(|| env_path("HOME").map(|home| home.join(".cache"))),
    };
    cache.filter(|dir| dir.is_absolute()).map(|dir| dir.join("ree"))
}

/// Read directories (searched in order, read-only) and optional write directory for an immutable
/// content-addressed store (`kind`, e.g. `"studio"` or `"reflection"`).
///
/// - Without `RUR_STATE_DIR` / `RUR_CACHE_DIR`: reads and writes share `<per_user_cache>/ree/shared/<kind>`
///   across every checkout, worktree, host, and standalone binary on the machine.
/// - With `RUR_STATE_DIR` / `RUR_CACHE_DIR`: reads check `<state_dir>/<kind>` first and then probe
///   `<per_user_cache>/ree/shared/<kind>` read-only (never creating dirs or writing outside the override);
///   writes go strictly to `<state_dir>/<kind>`.
/// - Setting `REE_SHARED_CACHE=0` disables the per-user shared directory.
pub(crate) fn shared_store_dirs(repo: &Path, kind: &str) -> (Vec<PathBuf>, Option<PathBuf>) {
    let has_override = std::env::var_os("RUR_STATE_DIR").is_some_and(|v| !v.is_empty())
        || std::env::var_os("RUR_CACHE_DIR").is_some_and(|v| !v.is_empty());
    let allow_shared = std::env::var_os("REE_SHARED_CACHE").as_deref() != Some(std::ffi::OsStr::new("0"));
    let shared = allow_shared.then(|| per_user_ree_cache_dir().map(|d| d.join("shared").join(kind))).flatten();

    if has_override {
        let own = crate::try_state_dir(repo).ok().map(|d| d.join(kind));
        let mut reads = Vec::new();
        if let Some(dir) = &own {
            reads.push(dir.clone());
        }
        if let Some(dir) = shared {
            if !reads.iter().any(|existing| existing == &dir) {
                reads.push(dir);
            }
        }
        return (reads, own);
    }

    if let Some(dir) = shared {
        return (vec![dir.clone()], Some(dir));
    }
    if repo.as_os_str().is_empty() && !crate::is_standalone_binary() {
        return (Vec::new(), None);
    }
    match crate::try_state_dir(repo).ok().map(|d| d.join(kind)) {
        Some(dir) => (vec![dir.clone()], Some(dir)),
        None => (Vec::new(), None),
    }
}

/// Removes stale `.tmp` files (>5 min old) and keeps at most `max_files` newest files with `ext`
/// inside `dir`. Removal errors (e.g. a file mapped by another process) are ignored.
pub(crate) fn sweep_immutable_store_dir(dir: &Path, ext: &str, max_files: usize) {
    let Ok(read_dir) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();
    let stale_tmp = std::time::Duration::from_secs(300);
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        if name.ends_with(".tmp") {
            if now.duration_since(modified).unwrap_or_default() > stale_tmp {
                let _ = std::fs::remove_file(&path);
            }
        } else if name.ends_with(ext) {
            entries.push((modified, path));
        }
    }
    if entries.len() <= max_files {
        return;
    }
    crate::sort::by(&mut entries, |a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (_, old_path) in entries.into_iter().skip(max_files) {
        let _ = std::fs::remove_file(old_path);
    }
}

fn studio_cache_file(dir: &Path, hash: (u64, u64)) -> PathBuf {
    dir.join(format!("v{STUDIO_INDEX_VERSION}-{:016x}{:016x}.studio", hash.0, hash.1))
}

/// Loads `path`'s binary [`StudioIndex`], reusing an in-process entry when unchanged, else mapping
/// an immutable content-addressed `.studio` file from `read_dirs` (building and atomically renaming
/// into `write_dir` on miss, corrupt file, or older version). Never retains [`HierarchySnapshot`]
/// in memory, and never caches a recent mtime stamp that could hide a same-tick edit after the
/// recency window closes.
fn load_studio_index_in(path: &str, read_dirs: &[PathBuf], write_dir: Option<&Path>) -> Option<std::sync::Arc<StudioIndex>> {
    load_studio_index_in_at(path, read_dirs, write_dir, std::time::SystemTime::now)
}

fn load_studio_index_in_at(
    path: &str,
    read_dirs: &[PathBuf],
    write_dir: Option<&Path>,
    mut now: impl FnMut() -> std::time::SystemTime,
) -> Option<std::sync::Arc<StudioIndex>> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Entry = (String, Option<crate::files::Stamp>, (u64, u64), Arc<StudioIndex>);
    static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();

    let source_path = Path::new(path);
    crate::files::note(source_path);
    let stamp = crate::files::stamp(source_path);
    let stable_stamp = stamp.filter(|st| !stamp_is_recent_at(*st, now()));
    let cache = CACHE.get_or_init(Default::default);
    if let Some(st) = stable_stamp {
        if let Ok(entries) = cache.lock() {
            if let Some((_, _, _, found)) = entries.iter().find(|(p, s, _, _)| p == path && *s == Some(st)) {
                return Some(found.clone());
            }
        }
    }

    let hash = hash_file_128(source_path)?;
    if let Ok(mut entries) = cache.lock() {
        if let Some((_, cached_stamp, _, found)) = entries.iter_mut().find(|(p, _, h, _)| p == path && *h == hash) {
            if cached_stamp.is_none() && stable_stamp.is_some() {
                *cached_stamp = stable_stamp;
            }
            return Some(found.clone());
        }
    }

    for dir in read_dirs {
        let target = studio_cache_file(dir, hash);
        if let Some(index) = StudioIndex::open_mapped(&target) {
            let index = crate::alloc::long_lived(|| Arc::new(index));
            if let Ok(mut entries) = cache.lock() {
                entries.retain(|(p, _, _, _)| p != path);
                crate::alloc::long_lived(|| entries.push((path.to_string(), stable_stamp, hash, index.clone())));
            }
            return Some(index);
        }
    }

    // Cold build (or cache miss / invalid file on disk): parse JSON, encode binary, drop JSON immediately.
    let pre_stamp = crate::files::stamp(source_path);
    let raw_bytes = std::fs::read(source_path).ok()?;
    let actual_hash = hash_bytes_128(&raw_bytes);
    let snapshot: HierarchySnapshot = serde_json::from_slice(&raw_bytes).ok()?;
    let encoded = StudioIndex::encode(&snapshot);
    drop(snapshot);
    drop(raw_bytes);

    let index = if let Some(dir) = write_dir {
        let target = studio_cache_file(dir, actual_hash);
        let _ = std::fs::create_dir_all(dir);
        let _ = crate::files::write_atomic(&target, &encoded);
        sweep_immutable_store_dir(dir, ".studio", STUDIO_INDEX_MAX_FILES);
        StudioIndex::open_mapped(&target).or_else(|| StudioIndex::from_owned(encoded))?
    } else {
        StudioIndex::from_owned(encoded)?
    };
    let index = crate::alloc::long_lived(|| Arc::new(index));
    let actual_stable_stamp = crate::files::stamp(source_path).filter(|st| Some(*st) == pre_stamp && !stamp_is_recent_at(*st, now()));
    if let Ok(mut entries) = cache.lock() {
        entries.retain(|(p, _, _, _)| p != path);
        crate::alloc::long_lived(|| entries.push((path.to_string(), actual_stable_stamp, actual_hash, index.clone())));
    }
    Some(index)
}

fn load_studio_index(engine: &Engine, path: &str) -> Option<std::sync::Arc<StudioIndex>> {
    let (read_dirs, write_dir) = shared_store_dirs(&engine.options.repo, "studio");
    load_studio_index_in(path, &read_dirs, write_dir.as_deref())
}

/// The Studio hierarchy snapshot of the place the roots belong to (`<project>/<PLACES_FILE>`).
pub fn find_hierarchy_snapshot(roots: &[String]) -> Option<String> {
    for root in roots {
        let normalized = normalize(root);
        let project_root = parent_path(&normalized);
        if project_root.is_empty() {
            continue;
        }
        let Some(config) = read_json(&format!("{project_root}/{PLACES_FILE}")) else { continue };
        let Some(environments) = config.get("environments").and_then(|e| e.as_array()) else { continue };
        for environment in environments {
            let project_file = environment.get("projectFile").and_then(|p| p.as_str()).map(normalize).unwrap_or_default();
            let Some(name) = environment.get("name").and_then(|n| n.as_str()) else { continue };
            if format!("{project_root}/{}", parent_path(&project_file)) != normalized {
                continue;
            }
            let committed = format!("{project_root}/{SNAPSHOTS_DIR}/{name}.json");
            crate::files::note(Path::new(&committed));
            if Path::new(&committed).is_file() {
                return Some(committed);
            }
        }
    }
    None
}

/// The place id a project declares for these roots ([`PLACES_FILE`] `studioPlaceId`).
pub fn declared_place_id(roots: &[String]) -> Option<f64> {
    for root in roots {
        let normalized = normalize(root);
        let project_root = parent_path(&normalized);
        if project_root.is_empty() {
            continue;
        }
        let Some(config) = read_json(&format!("{project_root}/{PLACES_FILE}")) else { continue };
        let Some(environments) = config.get("environments").and_then(|e| e.as_array()) else { continue };
        for environment in environments {
            let project_file = environment.get("projectFile").and_then(|p| p.as_str()).map(normalize).unwrap_or_default();
            if format!("{project_root}/{}", parent_path(&project_file)) == normalized {
                if let Some(id) = environment.get("studioPlaceId").and_then(|v| v.as_f64()) {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// rur.json at the project root: the first root folder with a value for `key` wins.
pub fn project_setting(roots: &[String], key: &str) -> Option<serde_json::Value> {
    for root in roots {
        let normalized = normalize(root);
        let trimmed = normalized.trim_end_matches('/');
        let (project_root, folder) = trimmed.rsplit_once('/')?;
        let Some(config) = read_json(&format!("{project_root}/rur.json")) else { continue };
        if let Some(value) = config.get(key).and_then(|values| values.get(folder)) {
            if !value.is_null() {
                return Some(value.clone());
            }
        }
    }
    None
}

// Filesystem scan (same listing order and directory checks as the Luau scan)

#[derive(Clone)]
struct ScanNode {
    path: String,
    entries: Vec<String>,
    /// The entries' stamps from the listing, when a process keeps compiled modules (they check
    /// a module file is unchanged without a query per file)
    stamps: Vec<Option<crate::files::Stamp>>,
    dirs: FxHashMap<String, ScanNode>,
    /// Entries that are links back to a directory on the way down to them (the entry, that directory's
    /// path): not listed, since the directory would hold itself without end
    loops: Vec<(String, String)>,
    /// A directory below was listed through a link, whose target a watch on this tree does not see
    linked: bool,
    /// A [`GIT_DIRECTORY`] below a listed one, left unlisted: it mounts as an empty Folder
    unlisted: bool,
}

/// The directory Git keeps a repository's own data in. Rojo builds it as any folder (`rojo build` 7.6.1 and 7.7.0-rc.1: a
/// Folder for each directory in it, and an instance for each file a sync rule takes, such as the JSON files tools leave
/// there), which is thousands of Folders for a repository with worktrees and remote branches. A scan lists none of it: a
/// `.git` below a mounted directory mounts as an empty Folder, reported ([`mount_plain_directory_as`]). A `.git` file (a
/// submodule's or a worktree's link to its repository) is a file no sync rule takes, in Rojo or here.
const GIT_DIRECTORY: &str = ".git";

/// A directory on the way a walk took to the one it lists, for finding a link back to one of them.
struct Above<'a> {
    path: &'a str,
    /// Its real path, the links resolved: found once, when a link below asks
    real: std::sync::OnceLock<Option<std::path::PathBuf>>,
    up: Option<&'a Above<'a>>,
}

impl<'a> Above<'a> {
    fn new(path: &'a str, up: Option<&'a Above<'a>>) -> Self {
        Above { path, real: std::sync::OnceLock::new(), up }
    }

    /// The path of the directory on this way that link `child` leads back to, if it does.
    fn looped_by(&self, child: &Path) -> Option<&'a str> {
        let target = std::fs::canonicalize(child).ok()?;
        let mut at = Some(self);
        while let Some(above) = at {
            if above.real.get_or_init(|| std::fs::canonicalize(above.path).ok()).as_deref() == Some(target.as_path()) {
                return Some(above.path);
            }
            at = above.up;
        }
        None
    }
}

/// Whether a scan checks entry `entry` for a directory: any but a Luau source's name. A name that starts with a dot is listed
/// and mounted as any other, as Rojo builds it (a package manager's folder, pesde's `Packages/.pesde`, which its link
/// modules require through), but for [`GIT_DIRECTORY`]'s contents.
fn may_be_directory(entry: &str) -> bool {
    !ends_with_ignore_ascii_case(entry, ".luau") && !ends_with_ignore_ascii_case(entry, ".lua")
}

/// Lists a directory tree (directories checked and scanned in parallel; listing order kept).
fn scan(path: String) -> ScanNode {
    scan_below(path, None)
}

/// [`scan`] of a directory the walk reached through the directories `up`.
fn scan_below(path: String, up: Option<&Above>) -> ScanNode {
    use rayon::prelude::*;
    let mut entries = Vec::new();
    let mut kinds = Vec::new();
    let mut stamps = Vec::new();
    let stamped = crate::lua::modules::keeping_between_runs() && !crate::lua::modules::is_batch_frozen();
    if let Ok(read) = std::fs::read_dir(&path) {
        let mut valid = true;
        for entry in read {
            let Ok(entry) = entry else {
                valid = false;
                break;
            };
            match entry.file_name().into_string() {
                Ok(name) => {
                    kinds.push(entry.file_type().ok());
                    if stamped {
                        stamps.push(crate::files::entry_stamp(&entry));
                    }
                    entries.push(name);
                }
                Err(_) => {
                    valid = false;
                    break;
                }
            }
        }
        if !valid {
            entries.clear();
            kinds.clear();
            stamps.clear();
        }
    }
    // Pre-filter candidate subdirectories without rayon overhead on leaf directories (the common case).
    let mut candidates: Vec<usize> = Vec::new();
    for (index, (entry, kind)) in entries.iter().zip(kinds.iter()).enumerate() {
        if !may_be_directory(entry) {
            continue;
        }
        let plain = kind.is_some_and(|k| !k.is_symlink()) && !entry.ends_with(['.', ' ']);
        if plain && !kind.unwrap().is_dir() {
            continue;
        }
        candidates.push(index);
    }
    if candidates.is_empty() {
        return ScanNode { path, entries, stamps, dirs: FxHashMap::default(), loops: Vec::new(), linked: false, unlisted: false };
    }
    let here = Above::new(&path, up);
    let visit = |index: usize| -> Option<(String, Result<ScanNode, String>)> {
        let entry = &entries[index];
        let kind = kinds[index];
        let child = format!("{path}/{entry}");
        // Lune's isDir on the joined path (follows links; Windows drops a trailing dot, so a
        // directory named "X." is not a directory to the Luau scan either). The listing's own
        // type says the same for any other entry, without a query per entry.
        let plain = kind.is_some_and(|kind| !kind.is_symlink()) && !entry.ends_with(['.', ' ']);
        let is_dir = match kind {
            Some(kind) if plain => kind.is_dir(),
            _ => std::fs::metadata(&child).is_ok_and(|m| m.is_dir()),
        };
        if !is_dir {
            return None;
        }
        if entry == GIT_DIRECTORY {
            return Some((entry.clone(), Ok(ScanNode { path: child, entries: Vec::new(), stamps: Vec::new(), dirs: FxHashMap::default(), loops: Vec::new(), linked: false, unlisted: true })));
        }
        let via_link = !kind.is_some_and(|kind| !kind.is_symlink());
        if let Some(above) = via_link.then(|| here.looped_by(Path::new(&child))).flatten() {
            return Some((entry.clone(), Err(above.to_string())));
        }
        let mut node = scan_below(child, Some(&here));
        node.linked |= via_link;
        Some((entry.clone(), Ok(node)))
    };
    let found: Vec<(String, Result<ScanNode, String>)> = if candidates.len() == 1 {
        visit(candidates[0]).into_iter().collect()
    } else {
        candidates.into_par_iter().filter_map(visit).collect()
    };
    let mut dirs = FxHashMap::default();
    let mut loops = Vec::new();
    for (entry, found) in found {
        match found {
            Ok(node) => {
                dirs.insert(entry, node);
            }
            Err(above) => loops.push((entry, above)),
        }
    }
    let linked = dirs.values().any(|dir| dir.linked);
    ScanNode { path, entries, stamps, dirs, loops, linked, unlisted: false }
}

#[test]
fn a_scan_lists_dot_directories_but_not_what_a_git_directory_below_holds() {
    let root = std::env::temp_dir().join(format!("ree-dot-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for file in [".hidden/value.luau", ".git/x.luau", ".git/objects/ab/cdef"] {
        std::fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
        std::fs::write(root.join(file), "return 1").unwrap();
    }
    let path = root.to_string_lossy().replace('\\', "/");
    let node = scan(path.clone());
    assert_eq!(node.dirs.get(".hidden").map(|dir| dir.entries.clone()), Some(vec!["value.luau".to_string()]));
    assert_eq!(node.dirs.get(".git").map(|git| (git.entries.len(), git.dirs.len())), Some((0, 0)), "nothing below a .git is listed");
    // (a .git mounted itself is listed: its mount names it)
    assert!(scan(format!("{path}/.git")).dirs.contains_key("objects"));
    std::fs::remove_dir_all(&root).unwrap();
}

/// Single-flight directory scan cache used while `ree run --batch` keeps the filesystem frozen:
/// parallel workers share one scan per root (including symlink/junction stages on Windows and Linux).
fn batch_listed(path: String) -> ScanNode {
    // Never block a rayon worker thread waiting on a scan that itself uses rayon.
    if rayon::current_thread_index().is_some() {
        return scan(path);
    }
    use std::sync::{Arc, Condvar, Mutex, OnceLock};
    type Slot = Arc<(Mutex<Option<ScanNode>>, Condvar)>;
    static BATCH_LISTINGS: OnceLock<Mutex<FxHashMap<String, Slot>>> = OnceLock::new();
    let map = BATCH_LISTINGS.get_or_init(Default::default);
    let (slot, owner) = {
        let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = guard.get(&path) {
            (existing.clone(), false)
        } else {
            let created: Slot = Arc::new((Mutex::new(None), Condvar::new()));
            guard.insert(path.clone(), created.clone());
            (created, true)
        }
    };
    if owner {
        let node = scan(path);
        let copy = crate::alloc::long_lived(|| node.clone());
        let (lock, cvar) = &*slot;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(copy);
        cvar.notify_all();
        node
    } else {
        let (lock, cvar) = &*slot;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        while guard.is_none() {
            guard = cvar.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        guard.as_ref().unwrap().clone()
    }
}

/// [`scan`] of a mounted tree, kept between VMs in a process that keeps compiled modules ([`kept_listing`]).
fn listed(path: String) -> ScanNode {
    if crate::lua::modules::is_batch_frozen() {
        return batch_listed(path);
    }
    #[cfg(windows)]
    if crate::lua::modules::keeping_between_runs() {
        return kept_listing::listed(path);
    }
    scan(path)
}

/// Listings kept while Windows reports no change below them and their path names the same directory.
#[cfg(windows)]
mod kept_listing {
    use super::{FxHashMap, ScanNode, scan};
    use std::ffi::c_void;
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    use std::sync::{Mutex, OnceLock};

    #[repr(C)]
    #[derive(Default)]
    struct FileInformation {
        attributes: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        volume: u32,
        size: [u32; 2],
        links: u32,
        index_high: u32,
        index_low: u32,
    }

    unsafe extern "system" {
        fn FindFirstChangeNotificationW(path: *const u16, subtree: i32, filter: u32) -> *mut c_void;
        fn FindCloseChangeNotification(handle: *mut c_void) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetFileInformationByHandle(file: *mut c_void, information: *mut FileInformation) -> i32;
    }

    const WAIT_TIMEOUT: u32 = 0x102;
    const DRIVE_FIXED: u32 = 3;
    // File and directory names, attributes, sizes and last-write times anywhere below
    const CHANGES: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x10;

    struct Kept {
        watch: *mut c_void,
        id: (u32, u64),
        node: ScanNode,
    }

    // SAFETY: a watch handle is waited on and closed only under the map's lock
    unsafe impl Send for Kept {}

    static KEPT: OnceLock<Mutex<FxHashMap<String, Kept>>> = OnceLock::new();

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain([0]).collect()
    }

    /// The volume and file index of the directory `path` names now.
    fn directory_id(path: &str) -> Option<(u32, u64)> {
        // FILE_FLAG_BACKUP_SEMANTICS opens a directory; no access asked, every share allowed
        let file = std::fs::OpenOptions::new().access_mode(0).share_mode(7).custom_flags(0x0200_0000).open(path).ok()?;
        let mut information = FileInformation::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 || information.attributes & 0x10 == 0 {
            return None;
        }
        Some((information.volume, (u64::from(information.index_high) << 32) | u64::from(information.index_low)))
    }

    /// A change notification on `path`'s tree, on a local fixed drive only (a share may not report changes).
    fn watch(path: &str) -> Option<*mut c_void> {
        let absolute = std::path::absolute(path).ok()?;
        let absolute = absolute.to_str()?;
        let drive = absolute.as_bytes();
        if absolute.len() > 240 || drive.len() < 3 || drive[1] != b':' || !matches!(drive[2], b'\\' | b'/') {
            return None;
        }
        if unsafe { GetDriveTypeW(wide(&format!("{}:\\", drive[0] as char)).as_ptr()) } != DRIVE_FIXED {
            return None;
        }
        let handle = unsafe { FindFirstChangeNotificationW(wide(absolute).as_ptr(), 1, CHANGES) };
        (!handle.is_null() && handle as isize != -1).then_some(handle)
    }

    pub(super) fn listed(path: String) -> ScanNode {
        let Some(id) = directory_id(&path) else { return scan(path) };
        let kept = KEPT.get_or_init(Default::default);
        if let Ok(map) = kept.lock() {
            if let Some(entry) = map.get(&path).filter(|entry| entry.id == id && unsafe { WaitForSingleObject(entry.watch, 0) } == WAIT_TIMEOUT) {
                return entry.node.clone();
            }
        }
        // The watch starts before the listing, so a change made while it lists shows next time
        let Some(watch) = watch(&path) else { return scan(path) };
        let node = scan(path.clone());
        if node.linked {
            unsafe { FindCloseChangeNotification(watch) };
            return node;
        }
        let copy = crate::alloc::long_lived(|| node.clone());
        if let Ok(mut map) = kept.lock() {
            if let Some(old) = crate::alloc::long_lived(|| map.insert(path, Kept { watch, id, node: copy })) {
                unsafe { FindCloseChangeNotification(old.watch) };
            }
        } else {
            unsafe { FindCloseChangeNotification(watch) };
        }
        node
    }

    #[test]
    fn a_kept_listing_follows_every_change_below_it() {
        let root = std::env::temp_dir().join(format!("ree-kept-listing-{}", std::process::id()));
        let path = root.to_string_lossy().replace('\\', "/");
        std::fs::create_dir_all(root.join("Folder")).unwrap();
        std::fs::write(root.join("Folder/a.luau"), "return 1").unwrap();
        let names = |node: &ScanNode| node.dirs["Folder"].entries.clone();
        assert_eq!(names(&listed(path.clone())), ["a.luau"]);
        assert_eq!(names(&listed(path.clone())), ["a.luau"]);
        std::fs::write(root.join("Folder/b.luau"), "return 2").unwrap();
        assert_eq!(names(&listed(path.clone())), ["a.luau", "b.luau"]);
        std::fs::remove_file(root.join("Folder/a.luau")).unwrap();
        assert_eq!(names(&listed(path.clone())), ["b.luau"]);
        std::fs::rename(&root, root.with_extension("old")).unwrap();
        std::fs::create_dir_all(root.join("Folder")).unwrap();
        assert!(names(&listed(path.clone())).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(root.with_extension("old")).unwrap();
    }
}

/// A script file's instance name and class. Rojo strips a `.server` / `.client` suffix whole, so `.server.luau` is a Script
/// named "", and names a module by its file stem, so `.luau` is a ModuleScript named ".luau" (`rojo build` 7.6.1 and 7.7.0-rc.1).
fn script_info(file_name: &str) -> Option<(&str, &'static str)> {
    for (suffix, class) in SCRIPT_SUFFIXES {
        if ends_with_ignore_ascii_case(file_name, suffix) {
            let stem = &file_name[..file_name.len() - suffix.len()];
            return Some((if stem.is_empty() && *class == "ModuleScript" { file_name } else { stem }, class));
        }
    }
    None
}

/// Byte-borrowed wrapper around [`Str`] for zero-allocation `&[u8]` lookups in [`ChildIndex`].
#[derive(Clone)]
struct ChildKey(Str);

impl std::borrow::Borrow<[u8]> for ChildKey {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl PartialEq for ChildKey {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0.as_bytes() == other.0.as_bytes()
    }
}

impl Eq for ChildKey {}

impl std::hash::Hash for ChildKey {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_bytes().hash(state);
    }
}

/// First-match child index for mounting directories with many children in O(1) per entry.
struct ChildIndex {
    by_name: FxHashMap<ChildKey, InstanceId>,
}

impl ChildIndex {
    fn new(state: &State, parent: InstanceId, reserve: usize) -> Self {
        let existing = &state.dom.at(parent).children;
        let mut by_name = FxHashMap::with_capacity_and_hasher(existing.len() + reserve, Default::default());
        for &child in existing {
            by_name.entry(ChildKey(state.dom.at(child).name.clone())).or_insert(child);
        }
        ChildIndex { by_name }
    }

    #[inline]
    fn get(&self, name: &str) -> Option<InstanceId> {
        self.by_name.get(name.as_bytes()).copied()
    }

    #[inline]
    fn insert(&mut self, name: Str, id: InstanceId) {
        self.by_name.insert(ChildKey(name), id);
    }

    #[inline]
    fn insert_if_absent(&mut self, name: Str, id: InstanceId) {
        self.by_name.entry(ChildKey(name)).or_insert(id);
    }
}

/// EnsureChild: the existing child for a mounted entry, or a new one (scripts replace plain
/// folders of an earlier root, keeping their children).
fn ensure_child(state: &mut State, reflection: &Reflection, parent: InstanceId, name: &str, class_name: &str, mount: &mut MountCounts) -> InstanceId {
    ensure_child_indexed(state, reflection, parent, name, class_name, mount, None)
}

fn ensure_child_indexed(
    state: &mut State,
    reflection: &Reflection,
    parent: InstanceId,
    name: &str,
    class_name: &str,
    mount: &mut MountCounts,
    mut index: Option<&mut ChildIndex>,
) -> InstanceId {
    let found = match &index {
        Some(idx) => idx.get(name),
        None => find_child(state, parent, name),
    };
    if let Some(existing) = found {
        let existing_class = state.dom.at(existing).class;
        if &*reflection.class(existing_class).name == class_name {
            return existing;
        }
        let existing_is_script = is_a(reflection, existing_class, "LuaSourceContainer");
        let wants_script = reflection.class_id(class_name).is_some_and(|c| is_a(reflection, c, "LuaSourceContainer"));
        if !existing_is_script && !wants_script {
            return existing;
        }
        let replaces_folder = !existing_is_script && wants_script && &*reflection.class(existing_class).name == "Folder";
        if (existing_is_script && wants_script) || replaces_folder {
            let class = reflection.class_id(class_name).expect("script class");
            let replacement = core::create_named(state, reflection, class, name);
            state.dom.hand_children(existing, replacement);
            state.dom.replace_child(parent, existing, replacement);
            if let Some(idx) = index.as_deref_mut() {
                idx.insert(state.dom.at(replacement).name.clone(), replacement);
            }
            return replacement;
        }
        if wants_script {
            mount.conflicts += 1;
        }
        return existing;
    }
    let class = reflection.class_id(class_name).expect("mounted class");
    let created = core::create_named(state, reflection, class, name);
    state.dom.attach(created, parent);
    if let Some(idx) = index.as_deref_mut() {
        idx.insert(state.dom.at(created).name.clone(), created);
    }
    mount.created += 1;
    created
}

fn mount_directory(engine: &Engine, node: &ScanNode, instance: InstanceId, mount: &mut MountCounts, relative: &str) {
    let has_loops = !node.loops.is_empty();
    let has_dirs = !node.dirs.is_empty();
    let mut child_index = {
        let state = engine.state.borrow();
        let existing_len = state.dom.at(instance).children.len();
        (node.entries.len() + existing_len >= 16).then(|| ChildIndex::new(&state, instance, node.entries.len()))
    };
    for entry in &node.entries {
        if has_loops {
            if let Some((_, above)) = node.loops.iter().find(|(name, _)| name == entry) {
                engine.warn(format!("{}/{entry}: a link back to {above}, which would hold itself; not mounted", node.path));
                continue;
            }
        }
        if has_dirs {
            if let Some(child_node) = node.dirs.get(entry) {
                let child_relative = if relative.is_empty() { entry.clone() } else { format!("{relative}/{entry}") };
                match nested_project(engine, child_node) {
                    // Named by the project's `name`, else by its directory (Rojo's src/project.rs, Project::set_file_name)
                    Some(project) => {
                        let name = project.get("name").and_then(serde_json::Value::as_str).unwrap_or(entry);
                        mount_nested_project(engine, child_node, &project, instance, name, mount, &child_relative, child_index.as_mut());
                    }
                    None => {
                        mount_plain_directory_as(engine, child_node, instance, entry, mount, &child_relative, child_index.as_mut());
                    }
                }
                continue;
            }
        }
        if starts_with_ignore_ascii_case(entry, "init.") || ends_with_ignore_ascii_case(entry, ".meta.json") {
            continue;
        }
        let full_path = format!("{}/{entry}", node.path);
        mount_file(engine, &full_path, entry, None, instance, mount, child_index.as_mut());
    }
}

/// A scanned directory mounted as the child `name` of `parent` (a project node's name, which Rojo keeps for a
/// `$path` target): as the project its `default.project.json` holds, else by its own init scripts and contents.
fn mount_directory_as(
    engine: &Engine,
    node: &ScanNode,
    parent: InstanceId,
    name: &str,
    mount: &mut MountCounts,
    relative: &str,
    parent_index: Option<&mut ChildIndex>,
) -> InstanceId {
    match nested_project(engine, node) {
        Some(project) => mount_nested_project(engine, node, &project, parent, name, mount, relative, parent_index),
        None => mount_plain_directory_as(engine, node, parent, name, mount, relative, parent_index),
    }
}

/// A directory's `default.project.json`: Rojo mounts the directory as that project "instead of anything else in the
/// directory" (rojo.space/docs/v7/sync-details/#projects), init scripts included. Wally packages each ship one.
/// None, reported, for a file that isn't a project.
fn nested_project(engine: &Engine, node: &ScanNode) -> Option<serde_json::Value> {
    let file = node.entries.iter().find(|entry| entry.eq_ignore_ascii_case("default.project.json"))?;
    let path = format!("{}/{file}", node.path);
    let project = read_json(&path).filter(|project| project.get("tree").is_some_and(serde_json::Value::is_object));
    if project.is_none() {
        engine.warn(format!("{path}: not a project ree can read (JSON with a \"tree\" object); the directory mounts as it is"));
    }
    project
}

/// How many projects a project can sit inside, the depth where roblox-ui-renderer's cli/rojo.mjs stops
const MAX_PROJECT_NESTING: u32 = 32;
/// How many projects one mount follows, far more than a game's packages: a tree's child nodes can each `$path` to the same
/// project, which doubles the mounts at every level within [`MAX_PROJECT_NESTING`]
const MAX_PROJECTS_PER_MOUNT: u32 = 10_000;

/// A nested project's tree mounted as the child `name` of `parent`. One whose `$path` is refused or names nothing mounts
/// the directory as it is; one past a limit above is left an empty Folder, reported.
#[allow(clippy::too_many_arguments)]
fn mount_nested_project(
    engine: &Engine,
    node: &ScanNode,
    project: &serde_json::Value,
    parent: InstanceId,
    name: &str,
    mount: &mut MountCounts,
    relative: &str,
    mut parent_index: Option<&mut ChildIndex>,
) -> InstanceId {
    let stopped = if mount.project_depth > MAX_PROJECT_NESTING {
        Some(format!("project nesting too deep (past {MAX_PROJECT_NESTING} levels)"))
    } else if mount.projects >= MAX_PROJECTS_PER_MOUNT {
        Some(format!("more than {MAX_PROJECTS_PER_MOUNT} projects in one mount"))
    } else {
        None
    };
    if let Some(reason) = stopped {
        engine.warn(format!("{}/default.project.json: {reason}; not mounted", node.path));
        return ensure_child_indexed(&mut engine.state.borrow_mut(), &engine.reflection, parent, name, "Folder", mount, parent_index);
    }
    mount.projects += 1;
    mount.project_depth += 1;
    let mounted = project
        .get("tree")
        .and_then(serde_json::Value::as_object)
        .and_then(|tree| mount_project_node(engine, node, tree, parent, name, mount, relative, parent_index.as_deref_mut()));
    let instance = match mounted {
        Some(instance) => instance,
        None => mount_plain_directory_as(engine, node, parent, name, mount, relative, parent_index),
    };
    mount.project_depth -= 1;
    instance
}

/// Whether a `$path` (split into `segments`) could name something outside its project file's folder: absolute (a leading
/// slash, which a UNC path has, or a drive) or with a `..` segment. Project files can come from third-party packages.
fn leaves_project_folder(path: &str, segments: &[&str]) -> bool {
    path.starts_with(['/', '\\']) || matches!(path.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic()) || segments.contains(&"..")
}

/// The target of a `$path` split into `segments`, mounted as the child `name` of `parent`. It is looked up only in `base`'s
/// listing, which holds nothing outside the project's folder. None when the listing has no such target.
#[allow(clippy::too_many_arguments)]
fn mount_listed_path(
    engine: &Engine,
    base: &ScanNode,
    segments: &[&str],
    parent: InstanceId,
    name: &str,
    mount: &mut MountCounts,
    relative: &str,
    mut parent_index: Option<&mut ChildIndex>,
) -> Option<InstanceId> {
    let Some((last, folders)) = segments.split_last() else {
        return Some(mount_plain_directory_as(engine, base, parent, name, mount, relative, parent_index));
    };
    let folder = folders.iter().try_fold(base, |dir, segment| dir.dirs.get(*segment))?;
    if let Some(dir) = folder.dirs.get(*last) {
        return Some(mount_directory_as(engine, dir, parent, name, mount, relative, parent_index));
    }
    if !folder.entries.iter().any(|entry| entry == last) {
        return None;
    }
    let full_path = format!("{}/{last}", folder.path);
    mount_file(engine, &full_path, last, Some(name), parent, mount, parent_index.as_deref_mut());
    let instance = match parent_index {
        Some(idx) => idx.get(name)?,
        None => find_child(&engine.state.borrow(), parent, name)?,
    };
    Some(instance)
}

/// A node of a nested project's tree (`$path`s relative to `base`, the project file's directory) mounted as the child
/// `name` of `parent`, then its child nodes; `$properties` and `$attributes` are ignored, as cli/rojo.mjs does. None when
/// its `$path` is refused or names nothing, reported unless Rojo allows that (`{ "optional": ... }`).
#[allow(clippy::too_many_arguments)]
fn mount_project_node(
    engine: &Engine,
    base: &ScanNode,
    tree: &serde_json::Map<String, serde_json::Value>,
    parent: InstanceId,
    name: &str,
    mount: &mut MountCounts,
    relative: &str,
    parent_index: Option<&mut ChildIndex>,
) -> Option<InstanceId> {
    let optional = tree.get("$path").and_then(|value| value.get("optional")?.as_str());
    let instance = match optional.or_else(|| tree.get("$path")?.as_str()) {
        Some(path) => {
            let project_file = format!("{}/default.project.json", base.path);
            let segments: Vec<&str> = path.split(['/', '\\']).filter(|segment| !segment.is_empty() && *segment != ".").collect();
            if leaves_project_folder(path, &segments) {
                engine.warn(format!("{project_file}: $path {path} leaves the project's folder; not mounted"));
                return None;
            }
            let found = mount_listed_path(engine, base, &segments, parent, name, mount, relative, parent_index);
            if found.is_none() && optional.is_none() {
                engine.warn(format!("{project_file}: $path {path} names nothing ree can mount; not mounted"));
            }
            found?
        }
        None => {
            let reflection = &engine.reflection;
            let class_name = tree.get("$className").and_then(|value| value.as_str()).filter(|class_name| reflection.class_id(class_name).is_some_and(|class| !reflection.class(class).service)).unwrap_or("Folder");
            ensure_child_indexed(&mut engine.state.borrow_mut(), reflection, parent, name, class_name, mount, parent_index)
        }
    };
    for (child_name, child) in tree {
        if let (false, Some(child)) = (child_name.starts_with('$'), child.as_object()) {
            mount_project_node(engine, base, child, instance, child_name, mount, &format!("{relative}/{child_name}"), None);
        }
    }
    Some(instance)
}

/// A scanned directory mounted as the child `name` of `parent` (init scripts and init.meta.json
/// className decide its class; a Folder otherwise), then its contents.
fn mount_plain_directory_as(
    engine: &Engine,
    node: &ScanNode,
    parent: InstanceId,
    name: &str,
    mount: &mut MountCounts,
    relative: &str,
    parent_index: Option<&mut ChildIndex>,
) -> InstanceId {
    let reflection = &engine.reflection;
    let full_path = &node.path;
    if node.unlisted {
        engine.warn(format!("{full_path}: a Git repository's own directory; mounted as an empty Folder (Rojo builds what is in it)"));
    }
    let mut meta_entry: Option<&str> = None;
    // (priority in SCRIPT_SUFFIXES, entry index, class)
    let mut best_init: Option<(usize, usize, &'static str)> = None;
    for (index, entry) in node.entries.iter().enumerate() {
        if !starts_with_ignore_ascii_case(entry, "init.") {
            continue;
        }
        if entry.eq_ignore_ascii_case("init.meta.json") {
            meta_entry = Some(entry.as_str());
            continue;
        }
        let rest = &entry[4..];
        for (priority, &(suffix, class)) in SCRIPT_SUFFIXES.iter().enumerate() {
            if rest.eq_ignore_ascii_case(suffix) {
                if best_init.is_none_or(|(best_p, _, _)| priority <= best_p) {
                    best_init = Some((priority, index, class));
                }
                break;
            }
        }
    }
    let meta = meta_entry.and_then(|file| read_json(&format!("{full_path}/{file}")));
    let meta_class = meta.as_ref().and_then(|m| m.get("className")).and_then(|c| c.as_str());
    let init = best_init.map(|(_, index, class)| {
        let found = &node.entries[index];
        (format!("{full_path}/{found}"), class)
    });
    let known = match relative {
        "StarterPlayer/StarterPlayerScripts" => Some("StarterPlayerScripts"),
        "StarterPlayer/StarterCharacterScripts" => Some("StarterCharacterScripts"),
        _ => None,
    };
    let explicit_class = init.as_ref().map(|&(_, c)| c).or(meta_class).or(known);
    let implicit = explicit_class.is_none();
    let class_name = explicit_class.unwrap_or("Folder");
    let child = {
        let mut state = engine.state.borrow_mut();
        let child = ensure_child_indexed(&mut state, reflection, parent, name, class_name, mount, parent_index);
        if let Some((path, _)) = &init {
            let instance = state.dom.at_mut(child);
            instance.set_source_path(Some(Rc::from(path.as_str())));
            instance.set_source(None);
            mount.scripts += 1;
        }
        // (a directory that says no class of its own takes the place's, once its hierarchy is known: use_studio_hierarchy)
        if implicit {
            state.project.implicit_dirs.push(child);
        }
        child
    };
    mount_directory(engine, node, child, mount, relative);
    child
}

/// A file entry of a mounted directory (`file_name` its name on disk; `name` overrides the
/// instance name): scripts by suffix, JSON models, JSON modules, models. Other files are ignored.
fn mount_file(
    engine: &Engine,
    full_path: &str,
    file_name: &str,
    name: Option<&str>,
    instance: InstanceId,
    mount: &mut MountCounts,
    parent_index: Option<&mut ChildIndex>,
) {
    let reflection = &engine.reflection;
    if let Some((script_name, class_name)) = script_info(file_name) {
        let mut state = engine.state.borrow_mut();
        let child = ensure_child_indexed(&mut state, reflection, instance, name.unwrap_or(script_name), class_name, mount, parent_index);
        let class = state.dom.at(child).class;
        if is_a(reflection, class, "LuaSourceContainer") {
            let instance = state.dom.at_mut(child);
            instance.set_source_path(Some(Rc::from(full_path)));
            instance.set_source(None);
            mount.scripts += 1;
        }
    } else if ends_with_ignore_ascii_case(file_name, ".model.json") {
        // Rojo's JSON model (project/json_model.rs): one instance, named after the file
        match mounted_plan(engine, full_path) {
            Some(plan) => instantiate_model(engine, &plan, name, instance, parent_index),
            None => mount.skipped += 1,
        }
    } else if ends_with_ignore_ascii_case(file_name, ".json") {
        // JSON modules (Rojo / JSON-backed data) return the decoded value; read on require
        let module_name = name.unwrap_or(&file_name[..file_name.len() - 5]);
        let mut state = engine.state.borrow_mut();
        let child = ensure_child_indexed(&mut state, reflection, instance, module_name, "ModuleScript", mount, parent_index);
        let instance = state.dom.at_mut(child);
        instance.set_source_path(Some(Rc::from(full_path)));
        instance.set_source(None);
        instance.set_flag(flags::JSON_MODULE, true);
    } else if ends_with_ignore_ascii_case(file_name, ".rbxm") || ends_with_ignore_ascii_case(file_name, ".rbxmx") {
        if let Some(plan) = mounted_plan(engine, full_path) {
            instantiate_model(engine, &plan, name, instance, parent_index);
        }
    }
}

/// A mounted model file's instances made under `parent`; with `name` (Rojo: a model $path is one
/// instance, named by its project node) a single top instance takes that name.
fn instantiate_model(engine: &Engine, plan: &ModelPlan, name: Option<&str>, parent: InstanceId, parent_index: Option<&mut ChildIndex>) {
    let created = plan.instantiate(engine, Some(parent));
    if let (Some(name), [root]) = (name, created.as_slice()) {
        let mut state = engine.state.borrow_mut();
        core::raw_set_silent(&mut state, &engine.reflection, *root, K::Name as NameId, Value::String(Str::from(name)));
    }
    if let Some(idx) = parent_index {
        let state = engine.state.borrow();
        for &root in &created {
            idx.insert_if_absent(state.dom.at(root).name.clone(), root);
        }
    }
}

/// A model file's instances: a Studio model (.rbxm / .rbxmx), or Rojo's JSON model (.model.json),
/// whose top instance is named after the file (one that does not decode, or one Rojo would reject, is reported here),
/// and whether reading it noted anything (also reported here: [`decode_model`]). Their rotations are as a client
/// receives them (received.rs). None when it does not decode or holds nothing.
fn decode_model_file(engine: &Engine, path: &str, bytes: &[u8]) -> Option<(WeakDom, bool)> {
    let file_name = Path::new(path).file_name()?.to_string_lossy();
    let mut noted = false;
    let mut dom = if !file_name.to_ascii_lowercase().ends_with(".model.json") {
        match decode_model(bytes) {
            Ok((dom, notes)) => {
                noted = !notes.is_empty();
                warn_notes(engine, path, &notes);
                dom
            }
            Err(message) => {
                engine.warn(format!("{path}: {message}; not mounted"));
                return None;
            }
        }
    } else {
        match json_model::decode(bytes, &file_name[..file_name.len() - ".model.json".len()]) {
            Ok(dom) => dom?,
            Err(message) => {
                engine.warn(format!("{path}: {message}; not mounted"));
                return None;
            }
        }
    };
    received::as_received(&mut dom);
    Some((dom, noted))
}

/// A model file's load notes ([`decode_model`]) as warnings that name the file.
pub(crate) fn warn_notes(engine: &Engine, path: &str, notes: &[String]) {
    for note in notes {
        engine.warn(format!("{path}: {note}"));
    }
}

/// A mounted model file converted: read, decoded and converted, or, in a process that keeps compiled
/// modules (a service), the plan kept for the file while its size and modification time are unchanged
/// (a sim mounts the same bundle file run after run; one whose reading noted something is read again, so each run
/// reports it). None when it does not read or decode.
fn mounted_plan(engine: &Engine, path: &str) -> Option<std::sync::Arc<ModelPlan>> {
    type Kept = std::sync::Mutex<FxHashMap<String, (crate::files::Stamp, std::sync::Arc<ModelPlan>)>>;
    static KEPT: std::sync::OnceLock<Kept> = std::sync::OnceLock::new();
    let keep = crate::lua::modules::keeping_between_runs();
    crate::files::note(Path::new(path));
    let stamp = crate::files::stamp(Path::new(path));
    if keep {
        let kept = KEPT.get_or_init(Default::default);
        if let Some((kept_stamp, plan)) = kept.lock().ok().and_then(|map| map.get(path).cloned()) {
            if Some(kept_stamp) == stamp {
                return Some(plan);
            }
        }
    }
    let bytes = std::fs::read(path).ok()?;
    let (dom, noted) = decode_model_file(engine, path, &bytes)?;
    let plan = std::sync::Arc::new(ModelPlan::new(&engine.reflection, &dom, dom.root().children()));
    if let (true, Some(stamp), false) = (keep, stamp, noted) {
        let kept = KEPT.get_or_init(|| crate::alloc::long_lived(Kept::default));
        let plan = crate::alloc::long_lived(|| std::sync::Arc::new(ModelPlan::clone_compacted(&plan)));
        if let Ok(mut map) = kept.lock() {
            map.retain(|other, (other_stamp, _)| crate::files::stamp(Path::new(other)) == Some(*other_stamp));
            crate::alloc::long_lived(|| map.insert(path.to_string(), (stamp, plan.clone())));
        }
        return Some(plan);
    }
    Some(plan)
}

/// One explicit mount (`--mount Service/Name=<path>` / `--instance Service/Name=<ClassName>`,
/// cli/rojo.mjs translates Rojo project files into these): `segments` is the instance path below
/// the DataModel, starting with a service.
#[derive(Clone, Debug, PartialEq)]
pub struct Mount {
    pub segments: Vec<String>,
    pub target: MountTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MountTarget {
    /// A directory (its contents become the instance's children, as a Rojo $path) or a file
    Path(String),
    /// An instance of this class (Rojo $className nodes without a $path)
    Class(String),
}

impl Mount {
    /// `Service/Name/...=<value>`; `class` for --instance.
    pub fn parse(text: &str, class: bool) -> Result<Mount, String> {
        let flag = if class { "--instance" } else { "--mount" };
        let (path, value) = text.split_once('=').ok_or_else(|| format!("{flag} needs Service/Name=<{}>", if class { "ClassName" } else { "path" }))?;
        let segments: Vec<String> = path.split(['/', '\\']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
        if segments.is_empty() || value.trim().is_empty() {
            return Err(format!("{flag} {text}: an instance path and a value are needed"));
        }
        let target = if class { MountTarget::Class(value.trim().to_string()) } else { MountTarget::Path(value.to_string()) };
        Ok(Mount { segments, target })
    }
}

/// Applies one [`Mount`]: the service, Folders for missing intermediate instances, then the
/// directory / file / class instance at the last segment. A directory mounted at a service
/// merges its contents into the service.
pub fn mount_at(engine: &Engine, spec: &Mount) -> Result<MountCounts, String> {
    let mut mount = MountCounts::default();
    let reflection = &engine.reflection;
    let service_name = &spec.segments[0];
    if !reflection.class_by_name(service_name).is_some_and(|c| c.service) {
        return Err(format!("{}: {service_name} is not a service (paths start with one, e.g. ReplicatedStorage/{})", spec.segments.join("/"), spec.segments.join("/")));
    }
    let game = engine.world().game;
    let service = services::get_service(engine, game, service_name).map_err(|_| format!("{service_name}: not a service"))?;
    let (last, middle) = spec.segments[1..].split_last().map_or((None, &[][..]), |(last, middle)| (Some(last), middle));
    let mut parent = service;
    for segment in middle {
        let mut state = engine.state.borrow_mut();
        parent = ensure_child(&mut state, reflection, parent, segment, "Folder", &mut mount);
    }
    let relative = spec.segments.join("/");
    match &spec.target {
        MountTarget::Class(class_name) => {
            let Some(last) = last else { return Ok(mount) };
            let class = reflection.class_id(class_name).filter(|&c| !reflection.class(c).service);
            let class_name = if class.is_some() { class_name.as_str() } else { "Folder" };
            let mut state = engine.state.borrow_mut();
            ensure_child(&mut state, reflection, parent, last, class_name, &mut mount);
        }
        MountTarget::Path(path) => {
            let path = normalize(path).trim_end_matches('/').to_string();
            crate::files::note(Path::new(&path));
            let metadata = std::fs::metadata(&path).map_err(|e| format!("{}={path}: {e}", spec.segments.join("/")))?;
            if metadata.is_dir() {
                let node = listed(path);
                match last {
                    Some(last) => {
                        mount_directory_as(engine, &node, parent, last, &mut mount, &relative, None);
                    }
                    None => mount_directory(engine, &node, service, &mut mount, service_name),
                }
                keep_listing(engine, node);
            } else {
                let Some(last) = last else {
                    return Err(format!("{service_name}={path}: a file cannot be mounted as a service"));
                };
                let file_name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if ends_with_ignore_ascii_case(&file_name, ".rbx") {
                    // a Roblox archive mounted on its own, as a model file is (a Rojo tree holds none: Rojo does not
                    // read them, so mount_file leaves them out of directories)
                    if let Some(plan) = mounted_plan(engine, &path) {
                        instantiate_model(engine, &plan, Some(last), parent, None);
                    }
                } else {
                    mount_file(engine, &path, &file_name, Some(last), parent, &mut mount, None);
                }
            }
        }
    }
    Ok(mount)
}

/// The contents of directory `path` mounted as the children of `instance`, as a mounted root's
/// directories are (scripts by suffix, `init` scripts making modules with children, Folders, JSON
/// modules, models): a record harness's own modules.
pub fn mount_children(engine: &Engine, path: &str, instance: InstanceId) -> MountCounts {
    let mut mount = MountCounts::default();
    let node = listed(normalize(path).trim_end_matches('/').to_string());
    mount_directory(engine, &node, instance, &mut mount, "");
    keep_listing(engine, node);
    mount
}

/// Mounts `<Root>/<Service>/...` directories onto the matching services.
pub fn mount_layout_root(engine: &Engine, root: &str) -> MountCounts {
    mount_scanned_layout_root(engine, scan_layout_root(root, &engine.reflection))
}

/// Single-flight layout root cache used while `ree run --batch` keeps the filesystem frozen.
fn batch_layout_root(root: &str, reflection: &Reflection) -> Vec<(String, ScanNode)> {
    use std::sync::{Arc, Condvar, Mutex, OnceLock};
    type Slot = Arc<(Mutex<Option<Vec<(String, ScanNode)>>>, Condvar)>;
    static BATCH_ROOTS: OnceLock<Mutex<FxHashMap<String, Slot>>> = OnceLock::new();
    let map = BATCH_ROOTS.get_or_init(|| crate::alloc::long_lived(Default::default));
    let (slot, owner) = {
        let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = guard.get(root) {
            (existing.clone(), false)
        } else {
            let created: Slot = crate::alloc::long_lived(|| Arc::new((Mutex::new(None), Condvar::new())));
            crate::alloc::long_lived(|| guard.insert(root.to_string(), created.clone()));
            (created, true)
        }
    };
    if owner {
        let scanned = scan_layout_root_uncached(root, reflection);
        let copy = crate::alloc::long_lived(|| scanned.clone());
        let (lock, cvar) = &*slot;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(copy);
        cvar.notify_all();
        scanned
    } else {
        let (lock, cvar) = &*slot;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(scanned) = guard.as_ref() {
                return scanned.clone();
            }
            guard = cvar.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// Layout root `root`'s service directories, listed: what [`mount_layout_root`] mounts, in its order.
fn scan_layout_root(root: &str, reflection: &Reflection) -> Vec<(String, ScanNode)> {
    crate::files::note_listing(Path::new(root));
    if crate::lua::modules::is_batch_frozen() && rayon::current_thread_index().is_none() {
        return batch_layout_root(root, reflection);
    }
    scan_layout_root_uncached(root, reflection)
}

fn scan_layout_root_uncached(root: &str, reflection: &Reflection) -> Vec<(String, ScanNode)> {
    let mut services_found = Vec::new();
    if let Ok(read) = std::fs::read_dir(root) {
        for entry in read.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            if name.starts_with('.') || !reflection.class_by_name(&name).is_some_and(|c| c.service) {
                continue;
            }
            let kind = entry.file_type().ok();
            let plain = kind.is_some_and(|k| !k.is_symlink()) && !name.ends_with(['.', ' ']);
            let is_dir = match kind {
                Some(k) if plain => k.is_dir(),
                _ => std::fs::metadata(format!("{root}/{name}")).is_ok_and(|m| m.is_dir()),
            };
            if is_dir {
                services_found.push(name);
            }
        }
    }
    if services_found.len() > 1 && !crate::lua::modules::is_batch_frozen() {
        use rayon::prelude::*;
        services_found.into_par_iter().map(|name| {
            let node = listed(format!("{root}/{name}"));
            (name, node)
        }).collect()
    } else {
        services_found.into_iter().map(|name| {
            let node = listed(format!("{root}/{name}"));
            (name, node)
        }).collect()
    }
}

fn is_common_service_name(name: &str) -> bool {
    matches!(
        name,
        "Workspace"
            | "ReplicatedStorage"
            | "ReplicatedFirst"
            | "ServerScriptService"
            | "ServerStorage"
            | "StarterGui"
            | "StarterPack"
            | "StarterPlayer"
            | "Lighting"
            | "SoundService"
            | "Players"
            | "Teams"
            | "Chat"
            | "TextChatService"
            | "LocalizationService"
            | "MaterialService"
            | "TestService"
            | "VoiceChatService"
            | "CoreGui"
            | "GuiService"
            | "HttpService"
            | "RunService"
            | "CollectionService"
            | "TweenService"
            | "UserInputService"
            | "TextService"
            | "InsertService"
            | "JointsService"
            | "Debris"
            | "ContextActionService"
            | "MarketplaceService"
            | "ProximityPromptService"
            | "PathfindingService"
            | "PhysicsService"
            | "PolicyService"
            | "SocialService"
            | "BadgeService"
            | "GroupService"
            | "MemoryStoreService"
            | "MessagingService"
            | "DataStoreService"
            | "TeleportService"
            | "AssetService"
            | "ContentProvider"
            | "AvatarEditorService"
            | "AnalyticsService"
            | "LogService"
            | "ScriptContext"
    )
}

enum EarlyEntry {
    Scanned(String, ScanNode),
    Deferred(String),
}

fn scan_layout_root_early(root: &str) -> Vec<EarlyEntry> {
    crate::files::note_listing(Path::new(root));
    let mut dirs: Vec<(String, bool)> = Vec::new();
    if let Ok(read) = std::fs::read_dir(root) {
        for entry in read.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            if name.starts_with('.') {
                continue;
            }
            let kind = entry.file_type().ok();
            let plain = kind.is_some_and(|k| !k.is_symlink()) && !name.ends_with(['.', ' ']);
            let is_dir = match kind {
                Some(k) if plain => k.is_dir(),
                _ => std::fs::metadata(format!("{root}/{name}")).is_ok_and(|m| m.is_dir()),
            };
            if is_dir {
                let common = is_common_service_name(&name);
                dirs.push((name, common));
            }
        }
    }
    let common_count = dirs.iter().filter(|(_, c)| *c).count();
    if common_count > 1 && !crate::lua::modules::is_batch_frozen() {
        use rayon::prelude::*;
        dirs.into_par_iter()
            .map(|(name, common)| {
                if common {
                    let node = listed(format!("{root}/{name}"));
                    EarlyEntry::Scanned(name, node)
                } else {
                    EarlyEntry::Deferred(name)
                }
            })
            .collect()
    } else {
        dirs.into_iter()
            .map(|(name, common)| {
                if common {
                    let node = listed(format!("{root}/{name}"));
                    EarlyEntry::Scanned(name, node)
                } else {
                    EarlyEntry::Deferred(name)
                }
            })
            .collect()
    }
}

fn mount_scanned_layout_root(engine: &Engine, scanned: Vec<(String, ScanNode)>) -> MountCounts {
    let mut mount = MountCounts::default();
    let game = engine.world().game;
    for (name, node) in scanned {
        let Ok(service) = services::get_service(engine, game, &name) else { continue };
        mount_directory(engine, &node, service, &mut mount, &name);
        keep_listing(engine, node);
    }
    mount
}

/// Layout roots listed on the thread pool while the caller goes on (the mount lists them during
/// Engine.Start, which needs no files): [`Prescan::mount`] mounts each as [`mount_layout_root`] would.
/// A pool of the caller's thread alone lists them when the caller waits ([`listing`]).
pub struct Prescan {
    roots: Vec<(String, crate::sort::channel::Receiver<Vec<(String, ScanNode)>>)>,
    early_roots: Vec<(String, crate::sort::channel::Receiver<Vec<EarlyEntry>>)>,
    /// The data roots ([`data_roots`]) and their subtrees listed: (relative path, listing) in order
    data: Option<crate::sort::channel::Receiver<Vec<(String, ScanNode)>>>,
}

impl Prescan {
    pub fn start(roots: &[String], reflection: &std::sync::Arc<Reflection>) -> Prescan {
        Prescan::start_for(roots, roots, reflection)
    }

    /// Starts listing layout and data roots before [`Reflection`] has finished loading, so NTFS
    /// directory scans overlap with both reflection deserialization and VM startup. Common service
    /// directories are scanned eagerly; any uncommon top-level directory is checked against
    /// reflection and scanned in order during [`Prescan::mount`].
    pub(crate) fn start_early(roots: &[String], place_roots: &[String]) -> Prescan {
        if crate::lua::modules::is_batch_frozen() {
            return Prescan { roots: Vec::new(), early_roots: Vec::new(), data: None };
        }
        let given = place_roots.to_vec();
        let early_roots: Vec<_> = roots
            .iter()
            .map(|root| {
                let (send, receive) = crate::sort::channel::channel();
                let path = root.clone();
                rayon::spawn(move || drop(send.send(scan_layout_root_early(&path))));
                (root.clone(), receive)
            })
            .collect();
        let (send, receive) = crate::sort::channel::channel();
        rayon::spawn(move || {
            let listed: Vec<(String, ScanNode)> = data_roots(&given).into_iter().flat_map(|(root, paths)| paths.into_iter().map(move |path| (root.clone(), path))).map(|(root, path)| { let node = listed(format!("{root}/{path}")); (path, node) }).collect();
            drop(send.send(listed));
        });
        Prescan { roots: Vec::new(), early_roots, data: Some(receive) }
    }

    /// [`Prescan::start`] of layout roots `roots`, with the data roots of the place `place_roots` names
    /// ([`crate::mount::place_roots`]: a Rojo project file's folder when it is mounted without roots).
    pub fn start_for(roots: &[String], place_roots: &[String], reflection: &std::sync::Arc<Reflection>) -> Prescan {
        // In `ree run --batch`, OS worker threads share `batch_listed` directly instead of queuing
        // duplicate prescan jobs onto the global rayon pool.
        if crate::lua::modules::is_batch_frozen() {
            return Prescan { roots: Vec::new(), early_roots: Vec::new(), data: None };
        }
        let given = place_roots.to_vec();
        let layout: Vec<_> = roots
            .iter()
            .map(|root| {
                let (send, receive) = crate::sort::channel::channel();
                let (path, reflection) = (root.clone(), reflection.clone());
                rayon::spawn(move || drop(send.send(scan_layout_root(&path, &reflection))));
                (root.clone(), receive)
            })
            .collect();
        let (send, receive) = crate::sort::channel::channel();
        rayon::spawn(move || {
            let listed: Vec<(String, ScanNode)> = data_roots(&given).into_iter().flat_map(|(root, paths)| paths.into_iter().map(move |path| (root.clone(), path))).map(|(root, path)| { let node = listed(format!("{root}/{path}")); (path, node) }).collect();
            drop(send.send(listed));
        });
        Prescan { roots: layout, early_roots: Vec::new(), data: Some(receive) }
    }

    /// The data roots' subtrees mounted from their listings (as mount_subtree mounts each, in
    /// [`data_roots`]' order; listed now if they were not listed ahead).
    pub fn mount_data(&mut self, engine: &Engine, roots: &[String]) {
        let listed = self.data.take().and_then(listing);
        let listed: Vec<(String, ScanNode)> = listed.unwrap_or_else(|| data_roots(roots).into_iter().flat_map(|(root, paths)| paths.into_iter().map(move |path| (root.clone(), path))).map(|(root, path)| { let node = self::listed(format!("{root}/{path}")); (path, node) }).collect());
        for (relative, node) in listed {
            mount_scanned_subtree(engine, &relative, node);
        }
    }

    /// Layout root `root` mounted from its listing (listed now if it was not listed ahead).
    pub fn mount(&mut self, engine: &Engine, root: &str) -> MountCounts {
        if let Some(index) = self.early_roots.iter().position(|(path, _)| path == root)
            && let Some(entries) = listing(self.early_roots.remove(index).1)
        {
            let mut scanned = Vec::with_capacity(entries.len());
            for entry in entries {
                match entry {
                    EarlyEntry::Scanned(name, node) => {
                        if engine.reflection.class_by_name(&name).is_some_and(|c| c.service) {
                            scanned.push((name, node));
                        }
                    }
                    EarlyEntry::Deferred(name) => {
                        if engine.reflection.class_by_name(&name).is_some_and(|c| c.service) {
                            let node = listed(format!("{root}/{name}"));
                            scanned.push((name, node));
                        }
                    }
                }
            }
            return mount_scanned_layout_root(engine, scanned);
        }
        let listed = match self.roots.iter().position(|(path, _)| path == root) {
            Some(index) => listing(self.roots.remove(index).1),
            None => None,
        };
        mount_scanned_layout_root(engine, listed.unwrap_or_else(|| scan_layout_root(root, &engine.reflection)))
    }
}

/// What a [`Prescan`] job sent (None if it failed). A caller that is a thread of the pool the job went
/// to runs the jobs it queued there while it waits: a pool built on its caller's thread
/// (`use_current_thread`, rur's browser module, whose page has no other thread) runs a job that
/// thread spawned only when the thread yields to rayon, so blocking on the channel there waited forever.
fn listing<T: Send + 'static>(receive: crate::sort::channel::Receiver<T>) -> Option<T> {
    loop {
        match receive.try_recv() {
            Ok(listed) => return Some(listed),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if rayon::yield_local() != Some(rayon::Yield::Executed) {
                    return receive.recv().ok();
                }
            }
        }
    }
}

#[derive(Clone)]
struct CachedDataRoot {
    entries: Vec<(String, Vec<String>)>,
    external_dir: Option<String>,
    noted_paths: Vec<String>,
}

fn project_data_roots(project_root: &str) -> CachedDataRoot {
    let manifest = read_json(&format!("{project_root}/{DATA_ROOTS_FILE}"));
    let Some(paths) = manifest.as_ref().and_then(|m| m.get(DATA_ROOTS_KEY)).and_then(|p| p.as_array()) else {
        return CachedDataRoot { entries: Vec::new(), external_dir: None, noted_paths: Vec::new() };
    };
    let external = format!("{project_root}/external");
    crate::files::note_listing(Path::new(&external));
    let mut entries = Vec::new();
    let mut noted_paths = Vec::new();
    if let Ok(read) = std::fs::read_dir(&external) {
        for entry in read.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            let source = format!("{external}/{name}/src");
            let mut found = Vec::new();
            for path in paths {
                let text = match path {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let clean = text.strip_prefix(DATA_ROOTS_PREFIX).unwrap_or(&text).trim_end_matches('/').to_string();
                let full = format!("{source}/{clean}");
                crate::files::note(Path::new(&full));
                if !clean.is_empty() && Path::new(&full).is_dir() {
                    found.push(clean);
                }
                noted_paths.push(full);
            }
            if !found.is_empty() {
                entries.push((source, found));
            }
        }
    }
    CachedDataRoot { entries, external_dir: Some(external), noted_paths }
}

/// [`DATA_ROOTS_FILE`]'s [`DATA_ROOTS_KEY`] paths materialized from `<project>/external/<Name>/src`.
pub fn data_roots(roots: &[String]) -> Vec<(String, Vec<String>)> {
    use std::sync::{Mutex, OnceLock};
    static BATCH_DATA_ROOTS: OnceLock<Mutex<FxHashMap<String, CachedDataRoot>>> = OnceLock::new();
    let frozen = crate::lua::modules::is_batch_frozen();
    let mut result = Vec::new();
    let mut seen = FxHashSet::default();
    for root in roots {
        let project_root = parent_path(&normalize(root));
        if project_root.is_empty() || !seen.insert(project_root.clone()) {
            continue;
        }
        if frozen {
            let cache = BATCH_DATA_ROOTS.get_or_init(|| crate::alloc::long_lived(Default::default));
            if let Some(cached) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&project_root).cloned() {
                crate::files::note(Path::new(&format!("{project_root}/{DATA_ROOTS_FILE}")));
                if let Some(ext) = &cached.external_dir {
                    crate::files::note_listing(Path::new(ext));
                }
                for p in &cached.noted_paths {
                    crate::files::note(Path::new(p));
                }
                result.extend(cached.entries);
                continue;
            }
            let computed = project_data_roots(&project_root);
            let stored = crate::alloc::long_lived(|| computed.clone());
            crate::alloc::long_lived(|| {
                cache.lock().unwrap_or_else(|e| e.into_inner()).insert(project_root, stored);
            });
            result.extend(computed.entries);
        } else {
            result.extend(project_data_roots(&project_root).entries);
        }
    }
    result
}

/// The Studio names of mounted scripts and folders whose names a file system cannot hold (a colon, a
/// slash, a trailing dot), from a sync tool's `<project>/<STUDIO_NAME_MAP>` (`{"files": {"<root>/<Service>/
/// .../<file>": {"studioPathParts": [...]}}}`): each entry maps a file's path to its Studio path parts,
/// and every mounted instance on that path whose name differs takes its Studio name, as the sync gives
/// it in Studio. Entries of roots that are not mounted, or whose instances are missing, change nothing.
/// Returns how many instances were renamed.
pub fn apply_studio_names(engine: &Engine, roots: &[String]) -> usize {
    let mut renamed = 0;
    let mut seen = FxHashSet::default();
    for root in roots {
        let normalized = normalize(root).trim_end_matches('/').to_string();
        let project_root = parent_path(&normalized);
        let root_name = normalized.rsplit('/').next().unwrap_or_default().to_string();
        if project_root.is_empty() || root_name.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        let Some(map) = read_json(&format!("{project_root}/{STUDIO_NAME_MAP}")) else { continue };
        let Some(files) = map.get("files").and_then(|f| f.as_object()) else { continue };
        for (key, entry) in files {
            let disk: Vec<&str> = key.split('/').collect();
            let Some(studio) = entry.get("studioPathParts").and_then(|p| p.as_array()) else { continue };
            let studio: Vec<&str> = studio.iter().filter_map(|part| part.as_str()).collect();
            if disk.len() != studio.len() || disk.len() < 3 || disk[0] != root_name {
                continue;
            }
            renamed += rename_studio_path(engine, &disk[1..], &studio[1..]);
        }
    }
    renamed
}

/// One name-map entry below a root (`Service/.../<file>` on disk and in Studio): walks the mounted
/// instances by either name and renames those whose Studio name differs.
fn rename_studio_path(engine: &Engine, disk: &[&str], studio: &[&str]) -> usize {
    let file_part = |part: &str| -> String {
        let lower = part.to_ascii_lowercase();
        if lower.starts_with("init.") && script_info(part).is_some() {
            return String::new();
        }
        script_info(part).map(|(name, _)| name.to_string()).unwrap_or_else(|| part.to_string())
    };
    let game = engine.world().game;
    let Some(mut current) = services::find_service(engine, game, disk[0]) else { return 0 };
    let mut renamed = 0;
    let last = disk.len() - 1;
    for index in 1..disk.len() {
        let (disk_name, studio_name) = if index == last { (file_part(disk[index]), file_part(studio[index])) } else { (disk[index].to_string(), studio[index].to_string()) };
        if disk_name.is_empty() {
            break;
        }
        let mut state = engine.state.borrow_mut();
        let Some(child) = find_child(&state, current, &disk_name).or_else(|| find_child(&state, current, &studio_name)) else { return renamed };
        if state.dom.at(child).name.as_bytes() != studio_name.as_bytes() {
            core::raw_set_silent(&mut state, &engine.reflection, child, K::Name as NameId, Value::String(Str::from(studio_name.as_str())));
            renamed += 1;
        }
        current = child;
    }
    renamed
}

/// Mounts `<Root>/<RelativePath>` onto the instance at the same path below the DataModel.
pub fn mount_subtree(engine: &Engine, root: &str, relative: &str) -> MountCounts {
    mount_scanned_subtree(engine, relative, listed(format!("{root}/{relative}")))
}

fn mount_scanned_subtree(engine: &Engine, relative: &str, node: ScanNode) -> MountCounts {
    let mut mount = MountCounts::default();
    let segments: Vec<&str> = relative.split('/').collect();
    let reflection = &engine.reflection;
    if !reflection.class_by_name(segments[0]).is_some_and(|c| c.service) {
        return mount;
    }
    let game = engine.world().game;
    let Ok(mut current) = services::get_service(engine, game, segments[0]) else { return mount };
    for segment in &segments[1..] {
        let mut state = engine.state.borrow_mut();
        current = ensure_child(&mut state, reflection, current, segment, "Folder", &mut mount);
    }
    mount_directory(engine, &node, current, &mut mount, relative);
    keep_listing(engine, node);
    mount
}

fn stand_in_source(file: &str) -> Option<Rc<[u8]>> {
    match file {
        "ui-labs.luau" => Some(Rc::from(&include_bytes!("lua/packages/ui-labs.luau")[..])),
        _ => None,
    }
}

fn mount_stand_in(engine: &Engine, parent: InstanceId, name: &str, file: &str) -> bool {
    let reflection = &engine.reflection;
    let mut state = engine.state.borrow_mut();
    if find_child(&state, parent, name).is_some() {
        return false;
    }
    let Some(source) = stand_in_source(file) else { return false };
    let module = core::create_named(&mut state, reflection, reflection.class_id("ModuleScript").unwrap(), name);
    let instance = state.dom.at_mut(module);
    instance.set_source(Some(source));
    instance.set_stand_in(Some(Rc::from(file)));
    state.dom.attach(module, parent);
    true
}

/// Mounts package stand-ins where the place's Studio hierarchy has them (or, without a snapshot,
/// into ReplicatedStorage.Packages / ReplicatedStorage.*.Packages) unless the roots provide them.
pub fn mount_stand_in_packages(engine: &Engine, roots: &[String]) -> u32 {
    let mut mounted = 0;
    let game = engine.world().game;
    if let Some(snapshot) = find_hierarchy_snapshot(roots) {
        let Some(index) = load_studio_index(engine, &snapshot) else { return 0 };
        let limit = index.node_count();
        let mut ancestors = Vec::new();
        for &node in index.stand_ins() {
            let Ok(name) = std::str::from_utf8(index.name(node)) else { continue };
            let Some(file) = stand_in_file(name) else { continue };
            ancestors.clear();
            let mut cur = index.parent(node);
            let mut steps = 0usize;
            while cur != 0 && index.parent(cur) != 0 && steps < limit {
                ancestors.push(cur);
                cur = index.parent(cur);
                steps += 1;
            }
            let mut target = Some(game);
            for &ancestor in ancestors.iter().rev() {
                let state = engine.state.borrow();
                target = target.and_then(|t| state.dom.find_child(t, index.name(ancestor)));
            }
            if let Some(target) = target {
                let exists = find_child(&engine.state.borrow(), target, name).is_some();
                if !exists && mount_stand_in(engine, target, name, file) {
                    mounted += 1;
                }
            }
        }
        return mounted;
    }
    let replicated = find_child(&engine.state.borrow(), game, "ReplicatedStorage");
    let Some(replicated) = replicated else { return 0 };
    let candidates: Vec<InstanceId> = {
        let state = engine.state.borrow();
        let mut list: Vec<InstanceId> = find_child(&state, replicated, "Packages").into_iter().collect();
        for &child in &state.dom.at(replicated).children {
            if let Some(packages) = find_child(&state, child, "Packages") {
                list.push(packages);
            }
        }
        list
    };
    for folder in candidates {
        let exists = find_child(&engine.state.borrow(), folder, "ui-labs").is_some();
        if !exists && mount_stand_in(engine, folder, "ui-labs", "ui-labs.luau") {
            mounted += 1;
        }
    }
    mounted
}

/// Loads the Studio hierarchy of the place the roots belong to and serves Studio-only instances
/// from it (Hooks.Materialize). Returns the snapshot path used.
pub fn use_studio_hierarchy(engine: &Engine, roots: &[String]) -> Option<String> {
    let path = find_hierarchy_snapshot(roots)?;
    let index = load_studio_index(engine, &path)?;
    let mut sources = FxHashMap::default();
    for (_, file) in PACKAGE_STAND_INS {
        if let Some(source) = stand_in_source(file) {
            sources.insert(file.to_string(), source);
        }
    }
    let mut state = engine.state.borrow_mut();
    state.project.studio = Some(Studio { game: engine.world().game, index: index.clone() });
    state.project.stand_in_sources = sources;
    state.project.hierarchy_path = Some(path.clone());
    // A directory that holds only scripts stands for an instance the place keeps (a Part a script sits in, as a script
    // sync tool lays a place's scripts out: a published place holds those Parts while the source tree has only their
    // scripts): it takes the class the hierarchy gives its path, and is Studio-only otherwise (its properties are the
    // class's defaults; the place's own values are place content's: mount_place_content). A Folder, a script class or a
    // service there leaves it a Folder.
    let reflection = &engine.reflection;
    let folder = reflection.class_id("Folder");
    for id in std::mem::take(&mut state.project.implicit_dirs) {
        if state.dom.get(id).is_none() || Some(state.dom.at(id).class) != folder {
            continue;
        }
        let Some(node) = studio_node_of(&mut state, id) else { continue };
        let Some(class) = reflection.class_id(index.class_name(node)) else { continue };
        if Some(class) == folder || reflection.class(class).service || is_a(reflection, class, "LuaSourceContainer") {
            continue;
        }
        state.dom.set_class(reflection, id, class);
        state.dom.at_mut(id).set_flag(flags::STUDIO_ONLY, true);
    }
    Some(path)
}

/// The model files below place-content directory `dir`, reached through the directories `up`; a link
/// back to one of those directories is not followed, but noted in `loops` (the link, that directory).
fn place_content_files(dir: &Path, up: Option<&Above>, files: &mut Vec<PathBuf>, loops: &mut Vec<(String, String)>) {
    crate::files::note_listing(dir);
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let path = dir.to_string_lossy();
    let here = Above::new(&path, up);
    for entry in read.flatten() {
        let child = entry.path();
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if child.is_dir() {
            let via_link = !entry.file_type().is_ok_and(|kind| !kind.is_symlink());
            match via_link.then(|| here.looped_by(&child)).flatten() {
                Some(above) => loops.push((child.to_string_lossy().into_owned(), above.to_string())),
                None => place_content_files(&child, Some(&here), files, loops),
            }
        } else if name.ends_with(".rbxm") || name.ends_with(".rbxmx") || name.ends_with(".rbx") {
            files.push(child);
        }
    }
}

/// Place content: the instances a place stores that its hierarchy snapshot lists by name and class
/// only (a Studio-only Model is a default one: scale 1, no pivot, no sizes). `<repo>/data/place-content/
/// <Place>/<parent path>/*.rbxm` (or `.rbxmx`, `.rbx`) holds them as Studio serialized them: the directories below the place
/// are the path of the parent (`ReplicatedStorage/Shared/Rigs`), each file's
/// roots become its children and replace the snapshot's placeholders of the same names, and the
/// snapshot adds nothing below them (it can be older than the place). Returns how many roots were
/// mounted.
pub fn mount_place_content(engine: &Engine, hierarchy: &str) -> usize {
    // a standalone run has no checkout of its own: it reads place content only from a repo named with `--repo`
    let names_input_repo = !engine.options.repo.as_os_str().is_empty();
    if crate::is_standalone(&engine.options.repo) && !names_input_repo {
        return 0;
    }
    let Some(place) = Path::new(hierarchy).file_stem().map(|stem| stem.to_string_lossy().into_owned()) else { return 0 };
    let top = repo_path(engine, "data/place-content").join(&place);
    let mut files = Vec::new();
    let mut loops = Vec::new();
    place_content_files(&top, None, &mut files, &mut loops);
    for (link, above) in loops {
        engine.warn(format!("place content {}: a link back to {}, which would hold itself; not read", normalize(&link), normalize(&above)));
    }
    crate::sort::ord(&mut files);
    let mut mounted = 0;
    for file in files {
        let segments: Vec<String> = file.parent().and_then(|dir| dir.strip_prefix(&top).ok()).map(|rel| rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        let path = normalize(&file.to_string_lossy());
        let Some(service_name) = segments.first() else {
            engine.warn(format!("place content {path}: files go in the folder of their parent's path"));
            continue;
        };
        let game = engine.world().game;
        let Ok(mut parent) = services::get_service(engine, game, service_name) else {
            engine.warn(format!("place content {path}: {service_name} is not a service"));
            continue;
        };
        let mut missing = None;
        for segment in &segments[1..] {
            crate::lua::instance::materialize(engine, parent);
            match find_child(&engine.state.borrow(), parent, segment) {
                Some(child) => parent = child,
                None => {
                    missing = Some(segment.clone());
                    break;
                }
            }
        }
        if let Some(segment) = missing {
            engine.warn(format!("place content {path}: no {segment} in the place"));
            continue;
        }
        crate::lua::instance::materialize(engine, parent);
        let Some(plan) = mounted_plan(engine, &path) else {
            engine.warn(format!("place content {path}: does not read as a model file"));
            continue;
        };
        // (only the snapshot's placeholders give way, as imported bundles do)
        let names: FxHashSet<&[u8]> = plan.root_names().collect();
        let obsolete: Vec<InstanceId> = {
            let state = engine.state.borrow();
            state.dom.at(parent).children.iter().copied().filter(|&child| state.dom.at(child).has(flags::STUDIO_ONLY) && names.contains(state.dom.at(child).name.as_bytes())).collect()
        };
        // (a directory the hierarchy gave its class keeps what the source tree mounted in it: it moves into the stored
        // instance, a stored script of the same name giving way to the source tree's)
        let carried: Vec<(Str, Vec<InstanceId>)> = {
            let state = engine.state.borrow();
            obsolete
                .iter()
                .map(|&child| {
                    let kept = state.dom.at(child).children.iter().copied().filter(|&kid| !state.dom.at(kid).has(flags::STUDIO_ONLY)).collect();
                    (state.dom.at(child).name.clone(), kept)
                })
                .collect()
        };
        for child in obsolete {
            crate::lua::instance::set_parent(engine, child, None).ok();
        }
        for warning in plan.warnings() {
            engine.warn(warning.clone());
        }
        // (and the file is the whole subtree: the snapshot, which can be older than the place, adds nothing below it)
        let roots = plan.instantiate(engine, Some(parent));
        for (name, kids) in carried {
            let Some(root) = roots.iter().copied().find(|&root| engine.state.borrow().dom.at(root).name == name) else { continue };
            for kid in kids {
                let stored = {
                    let state = engine.state.borrow();
                    let kid_name = state.dom.at(kid).name.clone();
                    find_child(&state, root, &kid_name.to_str_lossy()).map(|stored| {
                        let script = |id: InstanceId| is_a(&engine.reflection, state.dom.at(id).class, "LuaSourceContainer");
                        (stored, script(stored) && script(kid))
                    })
                };
                match stored {
                    Some((stored, true)) => {
                        crate::lua::instance::set_parent(engine, stored, None).ok();
                    }
                    Some((_, false)) => continue,
                    None => {}
                }
                crate::lua::instance::set_parent(engine, kid, Some(root)).ok();
            }
        }
        let mut state = engine.state.borrow_mut();
        for &root in &roots {
            state.dom.at_mut(root).studio_node = StudioNode::None;
        }
        mounted += roots.len();
    }
    mounted
}

// Models and snapshots (rbx-dom -> engine instances)

/// A model or place file's instances, and what reading it noted. A Roblox archive (.rbx) is read as the standalone
/// .rbxl / .rbxm it holds, with the values it keeps in objects of their own written back first: rbx_binary reads the
/// stored scene without them and without an error, so a script's Source would come back empty (the format and the
/// reader: roblox-object-renderer's docs/archives.md). The notes are the archive reader's: content kept as objects
/// with no documented layout, which reads as empty, and what it read past. An archive it refuses is an error.
pub fn decode_model(bytes: &[u8]) -> Result<(WeakDom, Vec<String>), String> {
    let scene = match ror_archive::unwrap(bytes) {
        Ok(scene) => scene,
        // neither a Roblox file nor a ZIP: decoded as before, so the error is the parser's
        Err(ror_archive::Error::NotArchive) => return decode_scene(bytes).map(|dom| (dom, Vec::new())),
        Err(error) => return Err(format!("Roblox archive: {error}")),
    };
    let notes = archive_notes(&scene.report);
    decode_scene(&scene.bytes).map(|dom| (dom, notes))
}

/// What an archive's reader reported, as load warnings (the wording roblox-object-renderer gives them).
pub(crate) fn archive_notes(report: &ror_archive::Report) -> Vec<String> {
    let mut notes: Vec<String> = report
        .embedded_content
        .iter()
        .map(|content| {
            let plural = if content.values == 1 { "" } else { "s" };
            format!("{}.{}: {} value{plural} kept in the archive as content objects ({} bytes) that have no documented layout; read as empty", content.class, content.property, content.values, content.bytes)
        })
        .collect();
    notes.extend(report.warnings.iter().map(|warning| format!("Roblox archive: {warning}")));
    notes
}

/// A standalone binary or XML model or place.
fn decode_scene(bytes: &[u8]) -> Result<WeakDom, String> {
    if bytes.starts_with(b"<roblox!") {
        // Files from a newer Studio can store a property as a type our reflection database doesn't
        // expect (2026-09: Tags written as a SharedString); rbx_binary rejects the whole file. Retry
        // with that property left out of a copy of the database, so it's skipped (only non-rendering
        // metadata has done this so far).
        let mut first_error = None;
        let mut database: Option<rbx_reflection::ReflectionDatabase<'static>> = None;
        for _ in 0..8 {
            let result = match &database {
                Some(db) => rbx_binary::Deserializer::new().reflection_database(db).deserialize(bytes),
                None => rbx_binary::from_reader(bytes),
            };
            let error = match result {
                Ok(dom) => return Ok(dom),
                Err(error) => error.to_string(),
            };
            let Some((class, property)) = mismatched_property(&error) else { return Err(first_error.unwrap_or(error)) };
            let db = database.get_or_insert_with(|| rbx_reflection_database::get().expect("reflection database").clone());
            if !forget_property(db, &class, &property) {
                return Err(first_error.unwrap_or(error));
            }
            first_error.get_or_insert(error);
        }
        Err(first_error.unwrap_or_default())
    } else {
        // (properties rbx-dom's database does not know are read too, as rbx_binary reads them: a newer Studio's, which the
        // engine's reflection has)
        let options = rbx_xml::DecodeOptions::new().property_behavior(rbx_xml::DecodePropertyBehavior::ReadUnknown);
        rbx_xml::from_reader(bytes, options).map_err(|e| e.to_string())
    }
}

/// (class, property) of an rbx_binary "Type mismatch: Property Class.Name should be ..." error.
fn mismatched_property(error: &str) -> Option<(String, String)> {
    let rest = error.split("Type mismatch: Property ").nth(1)?;
    let (qualified, _) = rest.split_once(" should be ")?;
    let (class, property) = qualified.split_once('.')?;
    Some((class.to_string(), property.to_string()))
}

/// Removes `property` from the class that declares it (walking up from `class`); false if absent.
fn forget_property(db: &mut rbx_reflection::ReflectionDatabase<'static>, class: &str, property: &str) -> bool {
    let mut current = Some(class.to_string());
    while let Some(name) = current {
        let Some(descriptor) = db.classes.get_mut(name.as_str()) else { return false };
        if descriptor.properties.remove(property).is_some() {
            return true;
        }
        current = descriptor.superclass.as_ref().map(|s| s.to_string());
    }
    false
}

/// The enum the engine's reflection gives `class_name.property`, when it is an enum property.
pub(crate) fn engine_enum(reflection: &Reflection, class_name: &str, property: &str) -> Option<crate::reflection::EnumId> {
    let class = reflection.class_id(class_name)?;
    reflection.class_property(class, reflection.name_id(property)?)?.enum_type
}

/// The enum an rbx-dom Enum property holds (from rbx-dom's property descriptor, as Lune reads it).
fn dom_enum_name(class_name: &str, property: &str) -> Option<String> {
    let database = rbx_reflection_database::get().ok()?;
    let mut current = database.classes.get(class_name);
    while let Some(class) = current {
        if let Some(descriptor) = class.properties.get(property) {
            if let rbx_reflection::DataType::Enum(name) = &descriptor.data_type {
                return Some(name.to_string());
            }
            return None;
        }
        current = class.superclass.as_ref().and_then(|s| database.classes.get(s.as_ref()));
    }
    None
}

/// An rbx-dom Enum property value as the engine's enum item: by the engine's own reflection when it
/// gives the property an enum (data/reflection.json, a newer Studio than rbx-dom's database: an
/// InputBinding's Type and PrimaryModifier, KeyCode.MouseWheel), else by rbx-dom's descriptor, as
/// Lune reads it.
fn dom_enum_value(reflection: &Reflection, class_name: &str, property: &str, item: u32) -> Option<Value> {
    if let Some(enum_id) = engine_enum(reflection, class_name, property) {
        return reflection.enums[enum_id as usize].by_value.get(&(item as i64)).copied().map(Value::EnumItem);
    }
    let enum_name = dom_enum_name(class_name, property)?;
    let database = rbx_reflection_database::get().ok()?;
    let descriptor = database.enums.get(enum_name.as_str())?;
    let (item_name, _) = descriptor.items.iter().find(|(_, v)| **v == item)?;
    reflection.enum_item(&enum_name, item_name).map(Value::EnumItem)
}

/// ConvertLune: engine instances for the top-level instances of a decoded model (children of
/// `parent` when given), with properties converted up front.
pub fn convert_model(engine: &Engine, dom: &WeakDom, parent: Option<InstanceId>) -> Vec<InstanceId> {
    convert_instances(engine, dom, dom.root().children(), parent)
}

/// Loads a Studio snapshot whose top-level instances become children of `parent` (a service
/// root's children merge into the existing service), with rotations as a client receives them.
pub fn load_snapshot(engine: &Engine, path: &Path, parent: InstanceId) -> Result<Vec<InstanceId>, String> {
    crate::files::note(path);
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (mut dom, notes) = decode_model(&bytes)?;
    warn_notes(engine, &path.to_string_lossy(), &notes);
    received::as_received(&mut dom);
    let mut result = Vec::new();
    for &root in dom.root().children() {
        let source = dom.get_by_ref(root).unwrap();
        let existing = {
            let state = engine.state.borrow();
            find_child(&state, parent, &source.name).filter(|&e| {
                let class = state.dom.at(e).class;
                &*engine.reflection.class(class).name == source.class.as_str() && engine.reflection.class(class).service
            })
        };
        if let Some(existing) = existing {
            result.extend(convert_instances(engine, &dom, source.children(), Some(existing)));
        } else {
            result.extend(convert_instances(engine, &dom, &[root], Some(parent)));
        }
    }
    Ok(result)
}

/// Deserializes a model file into detached instances, with rotations as a client receives them.
pub fn load_model(engine: &Engine, path: &Path) -> Result<Vec<InstanceId>, String> {
    crate::files::note(path);
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (mut dom, notes) = decode_model(&bytes)?;
    warn_notes(engine, &path.to_string_lossy(), &notes);
    received::as_received(&mut dom);
    Ok(convert_model(engine, &dom, None))
}

/// Walks the DataModel for the ModuleScript mounted from `path` (main.luau FindModuleByPath).
pub fn find_module_by_path(engine: &Engine, path: &str) -> Option<InstanceId> {
    let wanted = normalize(path).to_lowercase();
    let state = engine.state.borrow();
    let mut stack = vec![engine.world().game];
    while let Some(node) = stack.pop() {
        let instance = state.dom.at(node);
        if let Some(source) = instance.source_path() {
            if normalize(source).to_lowercase() == wanted {
                return Some(node);
            }
        }
        // Depth-first in child order
        for &child in instance.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

pub fn repo_path(engine: &Engine, relative: &str) -> PathBuf {
    engine.options.repo.join(relative)
}

#[cfg(test)]
mod studio_index_tests {
    use super::*;
    use crate::{Emulator, Options, mount::ProjectSpec};

    fn unique_dir(tag: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("ree-studio-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_json(extra_child: &str) -> String {
        format!(
            r#"{{"nodes":[
                {{"name":"Place","className":"DataModel","parent":-1}},
                {{"name":"ReplicatedStorage","className":"ReplicatedStorage","parent":0}},
                {{"name":"Packages","className":"Folder","parent":1}},
                {{"name":"ui-labs","className":"ModuleScript","parent":2}},
                {{"name":"Workspace","className":"Workspace","parent":0}},
                {{"name":"{extra_child}","className":"Part","parent":4}},
                {{"name":"HiddenScript","className":"Script","parent":4}}
            ]}}"#
        )
    }

    fn load_studio_index_test(path: &str, cache: &Path) -> Option<std::sync::Arc<StudioIndex>> {
        load_studio_index_in(path, &[cache.to_path_buf()], Some(cache))
    }

    #[test]
    fn studio_index_store_rejects_truncated_bitflip_and_older_version_files() {
        let dir = unique_dir("corrupt");
        let cache = dir.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let snap = dir.join("Place.json");
        let json = sample_json("ArenaA");
        std::fs::write(&snap, &json).unwrap();
        let snap_str = snap.to_string_lossy().replace('\\', "/");
        let hash = hash_bytes_128(json.as_bytes());
        let cached = studio_cache_file(&cache, hash);

        // 1) Truncated file (writer died halfway)
        std::fs::write(&cached, b"REESTUD1\x02\x00\x00\x00truncated").unwrap();
        assert!(StudioIndex::open_mapped(&cached).is_none());
        let rebuilt = load_studio_index_test(&snap_str, &cache).expect("rebuilds over truncated file");
        assert_eq!(rebuilt.node_count(), 7);
        assert_eq!(rebuilt.stand_ins(), &[4]);
        assert_eq!(rebuilt.class_name(6), "Part");
        assert_eq!(rebuilt.name(6), b"ArenaA");
        // Disk file is now valid and directly mappable.
        let remapped = StudioIndex::open_mapped(&cached).expect("rebuilt cache maps cleanly");
        assert_eq!(remapped.name(6), b"ArenaA");

        // 2) Same-length bit flip inside payload (name bytes) must be caught by checksum
        let mut flipped = std::fs::read(&cached).unwrap();
        let pos = flipped.windows(6).position(|w| w == b"ArenaA").expect("ArenaA in payload");
        flipped[pos] ^= 0x01;
        let flipped_file = dir.join("bitflip.studio");
        std::fs::write(&flipped_file, &flipped).unwrap();
        assert!(StudioIndex::open_mapped(&flipped_file).is_none(), "same-length bit flip must fail checksum");

        // 3) Older layout version in header (v1 / v0 / old magic)
        for bad_ver in [0u32, 1u32] {
            let mut old_ver = std::fs::read(&cached).unwrap();
            old_ver[8..12].copy_from_slice(&bad_ver.to_le_bytes());
            let older_file = dir.join(format!("old-ver-{bad_ver}.studio"));
            std::fs::write(&older_file, &old_ver).unwrap();
            assert!(StudioIndex::open_mapped(&older_file).is_none(), "version {bad_ver} must be rejected");
        }
        let mut old_magic = std::fs::read(&cached).unwrap();
        old_magic[..8].copy_from_slice(b"REESTUD0");
        let older_magic_file = dir.join("old-magic.studio");
        std::fs::write(&older_magic_file, &old_magic).unwrap();
        assert!(StudioIndex::open_mapped(&older_magic_file).is_none(), "old magic must be rejected");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn studio_index_store_is_safe_under_concurrent_writers() {
        let dir = unique_dir("concurrent");
        let cache = dir.join("cache");
        let snap = dir.join("Place.json");
        std::fs::write(&snap, sample_json("ArenaC")).unwrap();
        let snap_str = snap.to_string_lossy().replace('\\', "/");

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let b = barrier.clone();
            let s = snap_str.clone();
            let c = cache.clone();
            handles.push(std::thread::spawn(move || {
                b.wait();
                let idx = load_studio_index_test(&s, &c).expect("concurrent load succeeds");
                assert_eq!(idx.node_count(), 7);
                assert_eq!(idx.name(6), b"ArenaC");
                assert_eq!(idx.class_name(6), "Part");
                assert_eq!(idx.stand_ins(), &[4]);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Verify no leftover temporary files remain beside the immutable store file.
        let leftover_tmp: Vec<_> = std::fs::read_dir(&cache)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftover_tmp.is_empty(), "leftover tmp files: {leftover_tmp:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn studio_index_store_invalidates_same_tick_edit_immediately_and_after_window() {
        let dir = unique_dir("same-tick");
        let cache = dir.join("cache");
        let snap = dir.join("Place.json");
        let first = sample_json("Arena1");
        let second = sample_json("Arena2");
        let third = sample_json("Arena3");
        assert_eq!(first.len(), second.len());
        assert_eq!(first.len(), third.len());

        std::fs::write(&snap, &first).unwrap();
        let orig_mtime = std::fs::metadata(&snap).unwrap().modified().unwrap();
        let snap_str = snap.to_string_lossy().replace('\\', "/");
        let idx1 = load_studio_index_test(&snap_str, &cache).unwrap();
        assert_eq!(idx1.name(6), b"Arena1");

        // Overwrite immediately with identical byte length AND identical mtime (exact same-tick edit).
        std::fs::write(&snap, &second).unwrap();
        std::fs::File::options().write(true).open(&snap).unwrap().set_modified(orig_mtime).unwrap();
        let idx2 = load_studio_index_test(&snap_str, &cache).unwrap();
        assert_eq!(idx2.name(6), b"Arena2", "immediate same-tick edit must not return stale data");

        // Rewrite again with identical length + original mtime, then query past the 2 s recency window.
        // Because the recent stamp was never cached, the lookup after the window must STILL see Arena3.
        std::fs::write(&snap, &third).unwrap();
        std::fs::File::options().write(true).open(&snap).unwrap().set_modified(orig_mtime).unwrap();
        let after_window = std::time::SystemTime::now() + std::time::Duration::from_secs(3);
        let idx3 = load_studio_index_in_at(&snap_str, &[cache.clone()], Some(&cache), || after_window).unwrap();
        assert_eq!(idx3.name(6), b"Arena3", "same-tick edit observed after recency window must not return stale data");

        // A cold build lasting over 2 s must sample the clock again after the build so a file whose
        // mtime is recent at build completion is not cached against the old content's hash.
        let slow_first = sample_json("Slow01");
        let slow_second = sample_json("Slow02");
        assert_eq!(slow_first.len(), slow_second.len());
        let slow_snap = dir.join("SlowPlace.json");
        let slow_str = slow_snap.to_string_lossy().replace('\\', "/");
        std::fs::write(&slow_snap, &slow_first).unwrap();
        let build_start = std::time::SystemTime::now() - std::time::Duration::from_secs(3);
        let build_end = std::time::SystemTime::now();
        let mut ticks = [build_start, build_end].into_iter();
        let idx_slow1 = load_studio_index_in_at(&slow_str, &[cache.clone()], Some(&cache), || ticks.next().unwrap_or(build_end)).unwrap();
        assert_eq!(idx_slow1.name(6), b"Slow01");
        let slow_mtime = std::fs::metadata(&slow_snap).unwrap().modified().unwrap();
        std::fs::write(&slow_snap, &slow_second).unwrap();
        std::fs::File::options().write(true).open(&slow_snap).unwrap().set_modified(slow_mtime).unwrap();
        let idx_slow2 = load_studio_index_in_at(&slow_str, &[cache.clone()], Some(&cache), || after_window).unwrap();
        assert_eq!(idx_slow2.name(6), b"Slow02", "post-build recency check must use post-build time");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn studio_index_store_sweeps_old_generations() {
        let dir = unique_dir("sweep");
        let cache = dir.join("cache");
        let snap = dir.join("Place.json");
        let snap_str = snap.to_string_lossy().replace('\\', "/");

        for i in 0..20u32 {
            std::fs::write(&snap, sample_json(&format!("Gen{i:03}"))).unwrap();
            let idx = load_studio_index_test(&snap_str, &cache).unwrap();
            assert_eq!(idx.name(6), format!("Gen{i:03}").as_bytes());
        }
        let studio_files: Vec<_> = std::fs::read_dir(&cache)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".studio"))
            .collect();
        assert!(
            studio_files.len() <= STUDIO_INDEX_MAX_FILES,
            "expected at most {STUDIO_INDEX_MAX_FILES} .studio files, found {}",
            studio_files.len()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash128_streaming_matches_single_shot() {
        let sample: Vec<u8> = (0..1027).map(|i| ((i * 37 + 11) & 0xff) as u8).collect();
        let expected = hash_bytes_128(&sample);
        for chunk_size in [1usize, 3, 7, 8, 9, 13, 64, 500] {
            let mut h = Hash128::new();
            for chunk in sample.chunks(chunk_size) {
                h.update(chunk);
            }
            assert_eq!(h.finish(), expected, "chunk_size={chunk_size}");
        }
    }

    #[test]
    fn studio_index_reads_shared_dir_without_writing_to_it() {
        let dir = unique_dir("shared-ro");
        let shared = dir.join("shared");
        let isolated = dir.join("isolated");
        let snap = dir.join("Place.json");
        std::fs::write(&snap, sample_json("SharedHit")).unwrap();
        let snap_str = snap.to_string_lossy().replace('\\', "/");

        // Populate shared store once.
        let idx1 = load_studio_index_test(&snap_str, &shared).unwrap();
        assert_eq!(idx1.name(6), b"SharedHit");

        // Now load using isolated write_dir + read-only shared_dir with a fresh file copy (different path key).
        let snap2 = dir.join("PlaceCopy.json");
        std::fs::write(&snap2, sample_json("SharedHit")).unwrap();
        let snap2_str = snap2.to_string_lossy().replace('\\', "/");
        let idx2 = load_studio_index_in(&snap2_str, &[isolated.clone(), shared.clone()], Some(&isolated)).unwrap();
        assert_eq!(idx2.name(6), b"SharedHit");
        // Isolated dir was NOT even created because the read-only shared dir hit!
        assert!(!isolated.exists(), "shared hit should not need to write into isolated dir");

        // Miss in shared dir writes ONLY to isolated dir, never to shared dir.
        let snap3 = dir.join("PlaceMiss.json");
        std::fs::write(&snap3, sample_json("IsolatedOnly")).unwrap();
        let snap3_str = snap3.to_string_lossy().replace('\\', "/");
        let shared_before: Vec<_> = std::fs::read_dir(&shared).unwrap().flatten().map(|e| e.file_name()).collect();
        let idx3 = load_studio_index_in(&snap3_str, &[isolated.clone(), shared.clone()], Some(&isolated)).unwrap();
        assert_eq!(idx3.name(6), b"IsolatedOnly");
        let shared_after: Vec<_> = std::fs::read_dir(&shared).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(shared_before, shared_after, "override miss must never write to shared dir");
        assert!(isolated.exists(), "override miss must write to isolated dir");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn studio_index_cold_and_warm_mount_stand_ins_and_materialize_identically() {
        let dir = unique_dir("parity");
        let cache = dir.join("cache");
        let place_root = dir.join("place_src");
        std::fs::create_dir_all(place_root.join("ReplicatedStorage/Packages")).unwrap();
        std::fs::create_dir_all(place_root.join("Workspace/ArenaX")).unwrap();
        std::fs::write(place_root.join("Workspace/ArenaX/Child.luau"), "return 1").unwrap();
        std::fs::create_dir_all(dir.join(SNAPSHOTS_DIR)).unwrap();
        std::fs::write(dir.join(SNAPSHOTS_DIR).join("Place.json"), sample_json("ArenaX")).unwrap();
        std::fs::write(
            dir.join(PLACES_FILE),
            r#"{"environments":[{"name":"Place","projectFile":"place_src/default.project.json"}]}"#,
        )
        .unwrap();

        let root_str = place_root.to_string_lossy().replace('\\', "/");
        let mut results = Vec::new();
        for _pass in 0..2 {
            // Pass 0 builds `.studio` cold; pass 1 hits the mmapped `.studio` file warm.
            let mut opts = Options::default();
            opts.repo = cache.clone();
            let spec = ProjectSpec { roots: vec![root_str.clone()], ..ProjectSpec::default() };
            let emu = Emulator::start(Engine::new(opts).unwrap(), &spec).unwrap();
            emu.run(
                "Check",
                r#"
                local pkgs = game:GetService("ReplicatedStorage"):FindFirstChild("Packages")
                assert(pkgs and pkgs:FindFirstChild("ui-labs"), "stand-in ui-labs missing")
                local arena = workspace:FindFirstChild("ArenaX")
                assert(arena and arena.ClassName == "Part", "implicit dir should take Part class from hierarchy")
                assert(workspace:FindFirstChild("HiddenScript") == nil, "Studio-only non-standin script must stay absent")
                print("ok", arena.ClassName, pkgs["ui-labs"].ClassName)
                "#,
            )
            .unwrap();
            let out: Vec<String> = emu.output().into_iter().map(|l| format!("{}:{}", l.kind, l.message)).collect();
            results.push(out);
        }
        assert_eq!(results[0], results[1], "cold vs warm mmap mount must match");
        assert!(results[0].iter().any(|l| l == "print:ok Part ModuleScript"), "unexpected output: {:?}", results[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
