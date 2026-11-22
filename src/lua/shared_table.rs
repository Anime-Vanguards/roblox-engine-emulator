//! SharedTable: Roblox's table for sharing data between Actors. It is a userdata here as in Roblox (`type` is "userdata",
//! `typeof` "SharedTable"), so the table functions refuse it and the VM names it in its errors. Measured in Studio 0.748, a
//! playing client:
//!
//! - Keys are strings, or integers from 0 to 2^32 - 1 (a float of an integer value is that integer; "1" and 1 are two keys).
//!   Any other key raises at the script's line, on a read as on a write: "SharedTable keys can only be strings or numbers."
//!   for a key of another type, "Numeric SharedTable keys must be nonnegative integers less than 2^32" for another number.
//! - Values: booleans, numbers, strings, vectors and Roblox's value types are kept as they are ([`KEPT`]), a buffer as a copy
//!   (and a new copy on every read), a SharedTable as itself. A table becomes a new SharedTable (each place it appears, its
//!   tables too, its metatable ignored; a cycle raises "Cannot convert a cyclic table to a SharedTable"). Instances,
//!   functions and the engine's other objects raise "<Name>s can not be stored in a SharedTable"; threads, proxies, Enum,
//!   Enums and RotationCurveKeys are dropped (the key reads nil). nil removes the key.
//! - A table is converted on the Lua stack above the call's own values (new's arguments, a write's object, key and value,
//!   update's arguments and its callback's result): a table inside another sits two slots above it (its key below it), in an
//!   array as in a dictionary, and one that would sit above slot 300 raises "Max lua stack size reached", without a position,
//!   once it is known not to close a cycle (a cycle that closes there raises the cyclic message). So `new(t)` takes tables
//!   nested 150 deep and a write 149 ([`STACK_LIMIT`], measured with arrays and dictionaries; the outermost table is never
//!   refused). SharedTables already made are kept as themselves and add no depth. As every other copy of a script's table, a
//!   conversion counts what it makes against ree's guard on one copy and stops past it ([`stored`]).
//! - A write checks the key, then whether the table is frozen ("SharedTable is frozen"), then the value. Only the key's errors
//!   and the library's argument errors carry the script's position.
//! - `for k, v in S` walks the entries as they were when the loop started: the integer keys ascending, then the string keys in
//!   the order of Roblox's hash trie ([`string_order`]).
//! - `#`, `next`, `pairs`, `rawget`, `setmetatable` and the `table` library raise as on any userdata; every SharedTable shares
//!   one metatable, which is locked; `S:size()` raises "attempt to call missing method 'size' of SharedTable", and tostring is
//!   "SharedTable: 0x" and 16 hex digits.
//! - The library (a read-only table): `new([table])`, `clone(S [, deep])` and `cloneAndFreeze(S [, deep])` (a shallow copy
//!   shares the SharedTables it holds, a deep one copies them, frozen too for cloneAndFreeze; a clone of a frozen table is not
//!   frozen), `isFrozen`, `size`, `clear`, `increment(S, key, delta)` (returns the old value) and `update(S, key, callback)`
//!   (the callback gets the old value and returns the new one; it runs where it cannot yield, and an error it raises reaches
//!   the caller as raised). Their messages are below, as measured.
//!
//! Where the contents live: in the engine, as Roblox keeps them in its own (numbers, booleans, vectors, strings and buffers as
//! data, the value types the engine has values for as those, a SharedTable it holds as a shared reference), and the userdata
//! is a handle, one per SharedTable while a script holds it (so `S.t == S.t`). A SharedTable holds none of mlua's reference
//! slots (lua/refs.rs). The entries are a persistent hash trie, as in Roblox ([`Trie`]): a shallow copy shares it, and a write
//! copies only the nodes on its path that a copy shares, so a shallow copy takes the same time at any size, and a write right
//! after one pays for the path it copies (measured: 0.47 to 1.16 us for the first write after cloning 10 to 10,000 string-keyed
//! entries, where an overwrite takes 0.08). The value types the
//! engine has no value for (Ray, Axes, TweenInfo, the keypoints, ...) wait in one engine table, a slot each, freed once no
//! SharedTable holds them. A SharedTable that holds itself is kept, while its handle is not: measured, a client collected the
//! handles of 300 SharedTables that held themselves and their 300 MB of buffers stayed (plain ones gave theirs back). Here it is
//! kept until the engine's end, which empties every SharedTable still alive ([`Store`]'s drop). However long a chain of
//! SharedTables a script builds (each holding the next), letting go of it and deep-copying it walk the chain in a loop, never
//! a call nested once a link (Studio deep-copies a chain 1,000 long).
//!
//! What their calls cost the work clock: cost.rs [`crate::cost::SHARED_NEW_STRING_ENTRY`] and the prices after it.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::ffi::c_void;
use std::rc::{Rc, Weak};

use mlua::prelude::*;
use rustc_hash::FxHashMap;

use super::bindable::{self, STACK_FULL, STACK_LIMIT};
use super::{Args, Flow, convert, lock_metatable, none, raise, raise_at_caller, raw, typeof_value, values, wrapped};
use crate::{Engine, cost, value::Value};

/// A SharedTable as a script holds it: a handle to its contents.
pub struct LSharedTable(Rc<Shared>);

/// A SharedTable: whether it is frozen, and its entries, whose nodes the copies made of it share. Its identity is this
/// allocation.
struct Shared {
    frozen: Cell<bool>,
    entries: RefCell<Entries>,
}

impl Shared {
    fn new(entries: Entries, frozen: bool) -> Rc<Shared> {
        Rc::new(Shared { frozen: Cell::new(frozen), entries: RefCell::new(entries) })
    }

    fn is_frozen(&self) -> bool {
        self.frozen.get()
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        release(std::mem::take(self.entries.get_mut()));
    }
}

/// Lets go of `entries` one SharedTable at a time: a SharedTable they were the last to hold is emptied in turn here, not
/// dropped inside this drop, so a chain of any length (each holding the next) never nests a drop once a link.
fn release(entries: Entries) {
    let mut pending = vec![entries];
    while let Some(entries) = pending.pop() {
        entries.drain(&mut |value| {
            if let Stored::Shared(shared) = value
                && let Some(mut shared) = Rc::into_inner(shared)
            {
                pending.push(std::mem::take(shared.entries.get_mut()));
            }
        });
    }
}

#[derive(Clone, Default)]
struct Entries {
    numbers: Trie<u32>,
    strings: Trie<Rc<[u8]>>,
}

/// A value as a SharedTable holds it.
#[derive(Clone)]
enum Stored {
    Boolean(bool),
    Number(f64),
    Vector(f32, f32, f32),
    String(Rc<[u8]>),
    /// A buffer's bytes: a copy goes in, and every read makes a new buffer of them
    Buffer(Rc<[u8]>),
    /// A value type the engine has a value for (CFrame, UDim2, Color3, EnumItem, Font, ...)
    Value(Value),
    /// Another value type, kept in the [`Store`]'s pool
    Pooled(Rc<PoolSlot>),
    Shared(Rc<Shared>),
}

#[derive(Clone)]
enum Key {
    Number(u32),
    String(Rc<[u8]>),
}

impl Entries {
    fn get(&self, key: &Key) -> Option<&Stored> {
        match key {
            Key::Number(n) => self.numbers.get(n),
            Key::String(s) => self.strings.get(s),
        }
    }

    /// Sets the entry at `key` (None removes it): the value it replaces, for the caller to drop once it has let go of the
    /// entries, and how many nodes it copied because another copy shared them ([`Trie::insert`]).
    fn set(&mut self, key: Key, value: Option<Stored>) -> (Option<Stored>, usize) {
        let mut copied = 0;
        let replaced = match (key, value) {
            (Key::Number(n), Some(value)) => self.numbers.insert(n, value, &mut copied),
            (Key::String(s), Some(value)) => self.strings.insert(s, value, &mut copied),
            (Key::Number(n), None) => self.numbers.remove(&n, &mut copied),
            (Key::String(s), None) => self.strings.remove(&s, &mut copied),
        };
        (replaced, copied)
    }

    fn len(&self) -> usize {
        self.numbers.len + self.strings.len
    }

    /// The entries in the order a loop walks them: the integer keys ascending, then the string keys in Roblox's order.
    fn ordered(&self) -> Vec<(Key, Stored)> {
        let mut numbers = Vec::with_capacity(self.numbers.len);
        self.numbers.walk(&mut numbers);
        let mut strings = Vec::with_capacity(self.strings.len);
        self.strings.walk(&mut strings);
        numbers.into_iter().map(|(n, value)| (Key::Number(n), value)).chain(strings.into_iter().map(|(s, value)| (Key::String(s), value))).collect()
    }

    /// The SharedTables held, by key.
    fn tables(&self) -> Vec<(Key, Rc<Shared>)> {
        let mut tables = Vec::new();
        self.numbers.visit(&mut |n, value| {
            if let Stored::Shared(shared) = value {
                tables.push((Key::Number(*n), shared.clone()));
            }
        });
        self.strings.visit(&mut |s, value| {
            if let Stored::Shared(shared) = value {
                tables.push((Key::String(s.clone()), shared.clone()));
            }
        });
        tables
    }

    /// Every value of the nodes no other copy shares ([`Trie::drain`]).
    fn drain(self, out: &mut impl FnMut(Stored)) {
        self.numbers.drain(out);
        self.strings.drain(out);
    }
}

/// The keys of a [`Trie`]: the bits it branches on, five a level.
trait TrieKey: Clone + PartialEq {
    /// Whether a node walks its slots in order, entries and subnodes mixed (integers, which so come ascending), or its
    /// entries first and then its subnodes (strings, as Roblox walks them)
    const IN_ORDER: bool;
    fn bits(&self) -> u64;
    /// The slot of `bits` at `level`; None past the last level, where the keys left have equal bits
    fn slot(bits: u64, level: u32) -> Option<u32>;
    /// The order of keys with equal bits
    fn order(&self, other: &Self) -> Ordering;
}

/// Integer keys branch on their own bits from the highest (two, then six levels of five), so walking the slots in order walks
/// the integers ascending.
impl TrieKey for u32 {
    const IN_ORDER: bool = true;

    fn bits(&self) -> u64 {
        u64::from(*self)
    }

    fn slot(bits: u64, level: u32) -> Option<u32> {
        match level {
            0 => Some((bits >> 30) as u32 & 3),
            1..=6 => Some((bits >> (30 - 5 * level)) as u32 & 31),
            _ => None,
        }
    }

    fn order(&self, other: &Self) -> Ordering {
        self.cmp(other)
    }
}

/// String keys branch on the 64-bit FNV-1a hash of their bytes, five bits a level from the lowest; a node's entries come
/// before its subnodes. Measured: 299 random keys, 40 keys sharing a prefix, and keys after deletes and re-adds all came out in
/// this order, whatever order they were added in. Keys whose hashes are equal (not measured) go by their bytes.
impl TrieKey for Rc<[u8]> {
    const IN_ORDER: bool = false;

    fn bits(&self) -> u64 {
        fnv1a(self)
    }

    fn slot(bits: u64, level: u32) -> Option<u32> {
        (level * 5 < 64).then(|| (bits >> (5 * level)) as u32 & 31)
    }

    fn order(&self, other: &Self) -> Ordering {
        (**self).cmp(&**other)
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// A persistent hash trie, as Roblox keeps a SharedTable's entries: a copy shares every node, and a write copies only the nodes
/// on its path that another copy shares. It is canonical (a CHAMP): a key sits at the shallowest level where its slot is its
/// own, whatever order keys came and went in, so the order it walks in is a function of its keys.
#[derive(Clone)]
struct Trie<K> {
    root: Option<Rc<Node<K>>>,
    len: usize,
}

impl<K> Default for Trie<K> {
    fn default() -> Self {
        Trie { root: None, len: 0 }
    }
}

/// A node of a [`Trie`]: 32 slots, each empty, an entry or a subnode. Past the last level its entries have no slots and go by
/// the keys' order.
#[derive(Clone)]
struct Node<K> {
    entry_slots: u32,
    node_slots: u32,
    entries: Vec<(K, Stored)>,
    nodes: Vec<Rc<Node<K>>>,
}

impl<K> Default for Node<K> {
    fn default() -> Self {
        Node { entry_slots: 0, node_slots: 0, entries: Vec::new(), nodes: Vec::new() }
    }
}

/// Where the slot `bit` goes among the slots `slots` holds.
fn below(slots: u32, bit: u32) -> usize {
    (slots & (bit - 1)).count_ones() as usize
}

impl<K: TrieKey> Trie<K> {
    fn get(&self, key: &K) -> Option<&Stored> {
        let bits = key.bits();
        let mut node = self.root.as_deref()?;
        let mut level = 0;
        loop {
            let Some(slot) = K::slot(bits, level) else {
                // (keys whose bits are all equal: a list kept in the keys' order)
                return node.entries.binary_search_by(|(other, _)| other.order(key)).ok().map(|at| &node.entries[at].1);
            };
            let bit = 1 << slot;
            if node.entry_slots & bit != 0 {
                let (other, value) = &node.entries[below(node.entry_slots, bit)];
                return (other == key).then_some(value);
            }
            if node.node_slots & bit == 0 {
                return None;
            }
            node = &node.nodes[below(node.node_slots, bit)];
            level += 1;
        }
    }

    /// Puts `value` at `key`: the value it replaces. `copied` counts the nodes on its path that another trie shared, which it
    /// copied first.
    fn insert(&mut self, key: K, value: Stored, copied: &mut usize) -> Option<Stored> {
        let bits = key.bits();
        let old = insert(self.root.get_or_insert_with(Rc::default), key, bits, value, 0, copied);
        if old.is_none() {
            self.len += 1;
        }
        old
    }

    /// Takes `key` out: its value, counting the nodes copied as [`Trie::insert`] does. Nothing is copied for a key the trie
    /// does not hold.
    fn remove(&mut self, key: &K, copied: &mut usize) -> Option<Stored> {
        self.get(key)?;
        let old = remove(self.root.as_mut()?, key, key.bits(), 0, copied)?;
        self.len -= 1;
        if self.len == 0 {
            self.root = None;
        }
        Some(old)
    }

    /// The entries in walk order.
    fn walk(&self, out: &mut Vec<(K, Stored)>) {
        if let Some(root) = &self.root {
            walk(root, 0, out);
        }
    }

    /// Every entry, in no particular order.
    fn visit(&self, f: &mut impl FnMut(&K, &Stored)) {
        let mut nodes: Vec<&Node<K>> = self.root.as_deref().into_iter().collect();
        while let Some(node) = nodes.pop() {
            for (key, value) in &node.entries {
                f(key, value);
            }
            nodes.extend(node.nodes.iter().map(|node| &**node));
        }
    }

    /// Every value of the nodes no other trie shares, handed to `out`; the nodes others share are only let go of. In a loop:
    /// a drop never nests in a trie's levels.
    fn drain(self, out: &mut impl FnMut(Stored)) {
        let mut nodes: Vec<Rc<Node<K>>> = self.root.into_iter().collect();
        while let Some(node) = nodes.pop() {
            if let Some(node) = Rc::into_inner(node) {
                for (_, value) in node.entries {
                    out(value);
                }
                nodes.extend(node.nodes);
            }
        }
    }
}

/// The node, to change: copied first when another trie shares it (counted in `copied`).
fn node_mut<'a, K: TrieKey>(node: &'a mut Rc<Node<K>>, copied: &mut usize) -> &'a mut Node<K> {
    if Rc::strong_count(node) > 1 {
        *copied += 1;
    }
    Rc::make_mut(node)
}

/// [`Trie::insert`] at a node `level` deep.
fn insert<K: TrieKey>(node: &mut Rc<Node<K>>, key: K, bits: u64, value: Stored, level: u32, copied: &mut usize) -> Option<Stored> {
    let node = node_mut(node, copied);
    let Some(slot) = K::slot(bits, level) else {
        return match node.entries.binary_search_by(|(other, _)| other.order(&key)) {
            Ok(at) => Some(std::mem::replace(&mut node.entries[at].1, value)),
            Err(at) => {
                node.entries.insert(at, (key, value));
                None
            }
        };
    };
    let bit = 1 << slot;
    if node.entry_slots & bit != 0 {
        let at = below(node.entry_slots, bit);
        if node.entries[at].0 == key {
            return Some(std::mem::replace(&mut node.entries[at].1, value));
        }
        // (two keys in one slot: a subnode for both, as many levels down as their slots stay the same)
        let (other, other_value) = node.entries.remove(at);
        node.entry_slots &= !bit;
        let mut child = Rc::default();
        let other_bits = other.bits();
        insert(&mut child, other, other_bits, other_value, level + 1, copied);
        insert(&mut child, key, bits, value, level + 1, copied);
        node.nodes.insert(below(node.node_slots, bit), child);
        node.node_slots |= bit;
        return None;
    }
    if node.node_slots & bit != 0 {
        let at = below(node.node_slots, bit);
        return insert(&mut node.nodes[at], key, bits, value, level + 1, copied);
    }
    node.entries.insert(below(node.entry_slots, bit), (key, value));
    node.entry_slots |= bit;
    None
}

/// [`Trie::remove`] at a node `level` deep: the key's value, or None (and nothing taken) when the node does not hold it. A
/// subnode left with one entry and no subnode gives the entry back to its parent, which keeps the trie canonical.
fn remove<K: TrieKey>(node: &mut Rc<Node<K>>, key: &K, bits: u64, level: u32, copied: &mut usize) -> Option<Stored> {
    let node = node_mut(node, copied);
    let Some(slot) = K::slot(bits, level) else {
        let at = node.entries.binary_search_by(|(other, _)| other.order(key)).ok()?;
        return Some(node.entries.remove(at).1);
    };
    let bit = 1 << slot;
    if node.entry_slots & bit != 0 {
        let at = below(node.entry_slots, bit);
        if node.entries[at].0 != *key {
            return None;
        }
        node.entry_slots &= !bit;
        return Some(node.entries.remove(at).1);
    }
    if node.node_slots & bit == 0 {
        return None;
    }
    let at = below(node.node_slots, bit);
    let old = remove(&mut node.nodes[at], key, bits, level + 1, copied)?;
    if node.nodes[at].node_slots == 0 && node.nodes[at].entries.len() == 1 {
        let child = node.nodes.remove(at);
        node.node_slots &= !bit;
        let entry = match Rc::try_unwrap(child) {
            Ok(mut child) => child.entries.remove(0),
            Err(child) => child.entries[0].clone(),
        };
        node.entries.insert(below(node.entry_slots, bit), entry);
        node.entry_slots |= bit;
    }
    Some(old)
}

fn walk<K: TrieKey>(node: &Node<K>, level: u32, out: &mut Vec<(K, Stored)>) {
    if K::IN_ORDER && K::slot(0, level).is_some() {
        let (mut entries, mut nodes) = (node.entries.iter(), node.nodes.iter());
        for slot in 0..32 {
            let bit = 1u32 << slot;
            if node.entry_slots & bit != 0 {
                out.extend(entries.next().cloned());
            } else if node.node_slots & bit != 0
                && let Some(child) = nodes.next()
            {
                walk(child, level + 1, out);
            }
        }
    } else {
        out.extend(node.entries.iter().cloned());
        for child in &node.nodes {
            walk(child, level + 1, out);
        }
    }
}

/// A pool slot of the [`Store`]: its value is dropped from the pool once the last SharedTable holding it lets go (at the
/// next value pooled; a drop may come from the collector, where no Lua may run).
struct PoolSlot {
    id: u32,
    released: Rc<RefCell<Vec<u32>>>,
}

impl Drop for PoolSlot {
    fn drop(&mut self) {
        if let Ok(mut released) = self.released.try_borrow_mut() {
            released.push(self.id);
        }
    }
}

/// The engine's part of every SharedTable: the handles (a weak-valued table, one per SharedTable while a script holds it), the
/// pool of the values the engine has no value of its own for, and every SharedTable made, to empty those still alive at the
/// engine's end.
struct Store {
    handles: LuaTable,
    pool: LuaTable,
    released: Rc<RefCell<Vec<u32>>>,
    free: RefCell<Vec<u32>>,
    next: Cell<u32>,
    tables: RefCell<Vec<Weak<Shared>>>,
    /// How long `tables` may grow before those let go of are taken out
    tables_kept: Cell<usize>,
}

/// The engine's end (its VM closing): every SharedTable still alive is emptied, so one that holds itself, which a session keeps
/// as Roblox's does, does not outlive the engine with what it holds (a host running engine after engine gets it back).
impl Drop for Store {
    fn drop(&mut self) {
        for table in std::mem::take(self.tables.get_mut()) {
            if let Some(table) = table.upgrade()
                && let Ok(mut entries) = table.entries.try_borrow_mut()
            {
                let entries = std::mem::take(&mut *entries);
                release(entries);
            }
        }
    }
}

impl Store {
    /// A new SharedTable, listed for the engine's end.
    fn table(&self, entries: Entries, frozen: bool) -> Rc<Shared> {
        let shared = Shared::new(entries, frozen);
        let mut tables = self.tables.borrow_mut();
        if tables.len() >= self.tables_kept.get() {
            tables.retain(|table| table.strong_count() > 0);
            self.tables_kept.set((tables.len() * 2).max(1024));
        }
        tables.push(Rc::downgrade(&shared));
        shared
    }

    fn put(&self, value: LuaValue) -> LuaResult<Rc<PoolSlot>> {
        // (what was let go: taken out before any Lua runs, which may collect and let go of more)
        let released = std::mem::take(&mut *self.released.borrow_mut());
        for id in released {
            self.pool.raw_set(id, LuaValue::Nil)?;
            self.free.borrow_mut().push(id);
        }
        let reused = self.free.borrow_mut().pop();
        let id = reused.unwrap_or_else(|| {
            self.next.set(self.next.get() + 1);
            self.next.get()
        });
        self.pool.raw_set(id, value)?;
        Ok(Rc::new(PoolSlot { id, released: self.released.clone() }))
    }

    /// The handle of `shared`: the one a script holds, else a new one.
    fn handle(&self, lua: &Lua, shared: &Rc<Shared>) -> LuaResult<LuaAnyUserData> {
        let key = Rc::as_ptr(shared) as usize as f64;
        if let LuaValue::UserData(handle) = self.handles.raw_get::<LuaValue>(key)?
            && handle.borrow::<LSharedTable>().is_ok_and(|held| Rc::ptr_eq(&held.0, shared))
        {
            return Ok(handle);
        }
        let handle = lua.create_any_userdata(LSharedTable(shared.clone()))?;
        self.handles.raw_set(key, handle.clone())?;
        Ok(handle)
    }
}

/// Entries a call handled, for its price: by their key's kind, and the SharedTables a deep clone copied.
#[derive(Default)]
struct Tally {
    strings: u64,
    integers: u64,
    tables: u64,
}

impl Tally {
    fn count(&mut self, key: &Key) {
        match key {
            Key::Number(_) => self.integers += 1,
            Key::String(_) => self.strings += 1,
        }
    }

    fn count_entries(&mut self, entries: &Entries) {
        self.integers += entries.numbers.len as u64;
        self.strings += entries.strings.len as u64;
    }

    /// What converting the counted entries costs (SharedTable.new from a table, or a table written to a key).
    fn converted_price(&self) -> f64 {
        self.strings as f64 * cost::SHARED_NEW_STRING_ENTRY + self.integers as f64 * cost::SHARED_NEW_INTEGER_ENTRY
    }
}

/// Charges the work clock `seconds`, and the profiler's account of the call (as `R.Charge` does).
fn charge(engine: &Engine, seconds: f64) {
    if seconds > 0.0 {
        let state = engine.state.borrow();
        state.sched.charge(seconds);
        if !state.sched.is_muted() {
            crate::boundary::add_cost(seconds);
        }
    }
}

/// What a write costs beyond the VM's call to it, as timed in a client (cost.rs [`cost::SHARED_OVERWRITE_STRING`] and after): an
/// overwrite, or a key added or taken out (`changed`), and the nodes it copied because a shallow copy shared them, by the key's kind.
fn write_price(key: &Key, changed: bool, copied: usize) -> f64 {
    let own = match (key, changed) {
        (Key::String(_), false) => cost::SHARED_OVERWRITE_STRING,
        (Key::String(_), true) => cost::SHARED_CHANGE_STRING,
        (Key::Number(_), false) => cost::SHARED_OVERWRITE_INTEGER,
        (Key::Number(_), true) => cost::SHARED_CHANGE_INTEGER,
    };
    own + copied_price(key, copied)
}

/// The price of the nodes a write copied because a shallow copy shared them.
fn copied_price(key: &Key, copied: usize) -> f64 {
    copied as f64
        * match key {
            Key::String(_) => cost::SHARED_COPIED_STRING_NODE,
            Key::Number(_) => cost::SHARED_COPIED_INTEGER_NODE,
        }
}

const KEY_TYPE: &str = "SharedTable keys can only be strings or numbers.";
const KEY_NUMBER: &str = "Numeric SharedTable keys must be nonnegative integers less than 2^32";
const FROZEN: &str = "SharedTable is frozen";
const CYCLIC: &str = "Cannot convert a cyclic table to a SharedTable";

/// The value types a SharedTable keeps as they are (measured, each read back as itself). Native vectors, booleans, numbers and
/// strings are kept too.
const KEPT: &[&str] = &[
    "Axes", "BrickColor", "CFrame", "Color3", "ColorSequence", "ColorSequenceKeypoint", "EnumItem", "Faces", "Font", "NumberRange",
    "NumberSequence", "NumberSequenceKeypoint", "Path2DControlPoint", "PhysicalProperties", "Ray", "Rect", "Region3", "Region3int16",
    "SecurityCapabilities", "TweenInfo", "UDim", "UDim2", "Vector2", "Vector2int16", "Vector3int16",
];

/// The value types a SharedTable drops without a word (measured: the key reads nil after the write).
const DROPPED: &[&str] = &["Enum", "Enums", "RotationCurveKey", "userdata"];

fn key_of(value: &LuaValue) -> Result<Key, Flow> {
    let number = match value {
        LuaValue::String(s) => return Ok(Key::String(s.as_bytes().to_vec().into())),
        LuaValue::Integer(n) => *n as f64,
        LuaValue::Number(n) => *n,
        _ => return raise_at_caller(KEY_TYPE),
    };
    if (0.0..4294967296.0).contains(&number) && number.fract() == 0.0 { Ok(Key::Number(number as u32)) } else { raise_at_caller(KEY_NUMBER) }
}

fn key_value(lua: &Lua, key: &Key) -> LuaResult<LuaValue> {
    Ok(match key {
        Key::Number(n) => LuaValue::Number(f64::from(*n)),
        Key::String(s) => LuaValue::String(lua.create_string(s)?),
    })
}

/// A key as the increment messages quote it.
fn key_text(key: &Key) -> String {
    match key {
        Key::Number(n) => n.to_string(),
        Key::String(s) => String::from_utf8_lossy(s).into_owned(),
    }
}

fn refused<T>(name: &str) -> Result<T, Flow> {
    raise(format!("{name}s can not be stored in a SharedTable"))
}

/// How far above a table one inside it sits on the Lua stack while it is converted: its key and itself, in an array as in a
/// dictionary (measured: `new` takes arrays and dictionaries alike nested 150 deep).
const NESTED: usize = 2;

/// How a value is being converted: the stack slot a table found sits at ([`converted`]), the tables open around it (to find a
/// cycle), the entries converted so far (for the price), and what the conversion has made against ree's guard on one copy.
struct Conversion {
    height: usize,
    open: Vec<*const c_void>,
    tally: Tally,
    made: bindable::Tally,
}

impl Conversion {
    /// A conversion from the call's frame: `height` values on the stack (`new`'s arguments; a write's object, key and value;
    /// `update`'s arguments and its callback's result), the outermost table at the top of them.
    fn new(height: usize) -> Conversion {
        Conversion { height, open: Vec::new(), tally: Tally::default(), made: bindable::Tally::default() }
    }
}

/// What a SharedTable holds for `value` (None: no entry), a table converted as [`converted`] says. A table is converted once
/// for each place it appears, as in Roblox, so one call may make far more than the table it was given: every value counts
/// against ree's guard on a copy (bindable.rs `Tally`: [`bindable::COPY_VALUES`] values, [`bindable::COPY_BYTES`] bytes of
/// strings, string keys and buffers), and one past it raises the guard's error, as every other copy of a script's table does.
fn stored(lua: &Lua, engine: &Engine, store: &Store, value: &LuaValue, conversion: &mut Conversion) -> Result<Option<Stored>, Flow> {
    let bytes = match value {
        LuaValue::String(s) => s.as_bytes().len(),
        LuaValue::Buffer(buffer) => buffer.len(),
        _ => 0,
    };
    if let Err(message) = conversion.made.add(bytes) {
        return raise(message);
    }
    Ok(Some(match value {
        LuaValue::Nil | LuaValue::Thread(_) | LuaValue::LightUserData(_) | LuaValue::Error(_) | LuaValue::Other(_) => return Ok(None),
        LuaValue::Boolean(b) => Stored::Boolean(*b),
        LuaValue::Integer(n) => Stored::Number(*n as f64),
        LuaValue::Number(n) => Stored::Number(*n),
        LuaValue::Vector(v) => Stored::Vector(v.x(), v.y(), v.z()),
        LuaValue::String(s) => Stored::String(s.as_bytes().to_vec().into()),
        LuaValue::Buffer(buffer) => Stored::Buffer(buffer.to_vec().into()),
        LuaValue::Function(_) => return refused("Function"),
        LuaValue::Table(table) => Stored::Shared(converted(lua, engine, store, table, conversion)?),
        LuaValue::UserData(ud) => {
            if let Ok(handle) = ud.borrow::<LSharedTable>() {
                return Ok(Some(Stored::Shared(handle.0.clone())));
            }
            if raw::instance_of_ud(ud).is_some() {
                return refused("Instance");
            }
            let name = typeof_value(engine, value);
            match name.as_str() {
                name if KEPT.contains(&name) => match convert::from_lua(value) {
                    Value::Lua(_) => Stored::Pooled(store.put(value.clone())?),
                    known => Stored::Value(known),
                },
                name if DROPPED.contains(&name) => return Ok(None),
                "RBXScriptSignal" => return refused("EventInstance"),
                "RBXScriptConnection" => return refused("Connection"),
                other => return refused(other),
            }
        }
    }))
}

/// A table as a new SharedTable (its own tables converted too), its entries counted as they go (also those walked before a
/// failure). A table inside another sits [`NESTED`] slots above it; one that is already open closes a cycle, and otherwise one
/// above [`STACK_LIMIT`] is too deep (in that order, measured: a cycle closing at the limit raises the cyclic message). The
/// outermost table is never refused.
fn converted(lua: &Lua, engine: &Engine, store: &Store, table: &LuaTable, conversion: &mut Conversion) -> Result<Rc<Shared>, Flow> {
    let pointer = table.to_pointer();
    if conversion.open.contains(&pointer) {
        return raise(CYCLIC);
    }
    if !conversion.open.is_empty() && conversion.height > STACK_LIMIT {
        return raise(STACK_FULL);
    }
    conversion.open.push(pointer);
    conversion.height += NESTED;
    let mut entries = Entries::default();
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair?;
        let key = key_of(&key)?;
        conversion.tally.count(&key);
        if let Key::String(bytes) = &key
            && let Err(message) = conversion.made.add_bytes(bytes.len())
        {
            return raise(message);
        }
        let value = stored(lua, engine, store, &value, conversion)?;
        entries.set(key, value);
    }
    conversion.height -= NESTED;
    conversion.open.pop();
    Ok(store.table(entries, false))
}

fn load(lua: &Lua, engine: &Engine, store: &Store, stored: &Stored) -> LuaResult<LuaValue> {
    Ok(match stored {
        Stored::Boolean(b) => LuaValue::Boolean(*b),
        Stored::Number(n) => LuaValue::Number(*n),
        Stored::Vector(x, y, z) => LuaValue::Vector(mlua::Vector::new(*x, *y, *z)),
        Stored::String(bytes) => LuaValue::String(lua.create_string(&**bytes)?),
        Stored::Buffer(bytes) => LuaValue::Buffer(lua.create_buffer(&**bytes)?),
        Stored::Value(value) => convert::to_lua(lua, engine, value)?,
        Stored::Pooled(slot) => store.pool.raw_get(slot.id)?,
        Stored::Shared(shared) => LuaValue::UserData(store.handle(lua, shared)?),
    })
}

/// A copy of `shared`, frozen or not. A shallow one shares its entries' trie (a write to either copies only its path); a deep
/// one copies the SharedTables they hold as well, each once (a table held twice, or holding itself, is copied once), counting
/// their entries and the tables in `tally`. A deep copy walks the tables with a stack of its own, so a chain of any length
/// never nests a call once a link.
fn copy(store: &Store, shared: &Rc<Shared>, deep: bool, frozen: bool, tally: &mut Tally) -> Rc<Shared> {
    let result = store.table(shared.entries.borrow().clone(), frozen);
    if !deep {
        return result;
    }
    let mut copies: FxHashMap<*const Shared, Rc<Shared>> = FxHashMap::default();
    copies.insert(Rc::as_ptr(shared), result.clone());
    let mut pending = vec![result.clone()];
    while let Some(copied) = pending.pop() {
        // (its entries are still the original's, shared: each SharedTable among them is swapped for its copy)
        let mut entries = copied.entries.borrow().clone();
        tally.count_entries(&entries);
        for (key, inner) in entries.tables() {
            tally.tables += 1;
            let inner_copy = copies
                .entry(Rc::as_ptr(&inner))
                .or_insert_with(|| {
                    let inner_copy = store.table(inner.entries.borrow().clone(), frozen);
                    pending.push(inner_copy.clone());
                    inner_copy
                })
                .clone();
            entries.set(key, Some(Stored::Shared(inner_copy)));
        }
        let original = copied.entries.replace(entries);
        drop(original);
    }
    result
}

/// The script-facing name of argument `index` (0-based) for an argument error: "no value" when it was not passed.
fn arg_type(engine: &Engine, args: &Args, index: usize) -> String {
    if index >= args.len() { "no value".into() } else { typeof_value(engine, args.get(index)) }
}

/// Argument `index` (0-based) of library function `function`, which must be a SharedTable: its contents.
fn shared_arg(engine: &Engine, args: &Args, index: usize, function: &str) -> Result<Rc<Shared>, Flow> {
    match args.get(index) {
        LuaValue::UserData(ud) if ud.is::<LSharedTable>() => Ok(ud.borrow::<LSharedTable>()?.0.clone()),
        _ => raise_at_caller(format!("invalid argument #{} to '{function}' (SharedTable expected, got {})", index + 1, arg_type(engine, args, index))),
    }
}

/// The SharedTable a metamethod was called on.
fn this(args: &Args) -> Result<Rc<Shared>, Flow> {
    match args.get(0) {
        LuaValue::UserData(ud) => Ok(ud.borrow::<LSharedTable>()?.0.clone()),
        _ => raise("invalid SharedTable"),
    }
}

/// The optional `deep` argument of clone and cloneAndFreeze: false when not passed, else a boolean.
fn deep_arg(engine: &Engine, args: &Args, function: &str) -> Result<bool, Flow> {
    match args.get(1) {
        _ if args.len() < 2 => Ok(false),
        LuaValue::Boolean(deep) => Ok(*deep),
        _ => raise_at_caller(format!("invalid argument #2 to '{function}' (boolean expected, got {})", arg_type(engine, args, 1))),
    }
}

/// The value at `key`, for a script.
fn read(lua: &Lua, engine: &Engine, store: &Store, shared: &Shared, key: &Key) -> LuaResult<LuaValue> {
    match shared.entries.borrow().get(key) {
        Some(stored) => load(lua, engine, store, stored),
        None => Ok(LuaValue::Nil),
    }
}

/// Writes `value` at `key` (the key already checked, the table not frozen). A table written is converted at `new`'s price,
/// also for the entries walked before a conversion fails.
/// `height`: the values on the stack below it ([`Conversion::new`]).
fn write(lua: &Lua, engine: &Engine, store: &Store, shared: &Shared, key: Key, value: &LuaValue, height: usize) -> Result<(), Flow> {
    let mut conversion = Conversion::new(height);
    let value = stored(lua, engine, store, value, &mut conversion);
    charge(engine, conversion.tally.converted_price());
    let value = value?;
    let (priced, adds) = (key.clone(), value.is_some());
    let (replaced, copied) = shared.entries.borrow_mut().set(key, value);
    let changed = adds != replaced.is_some();
    drop(replaced);
    charge(engine, write_price(&priced, changed, copied));
    Ok(())
}

/// SharedTable.update's checks and its callback's call, as (failed, problem, level): the wrapper in [`register`] raises
/// `problem` at the script's line (level 1) or as it is (level 0: the callback's own error, its value kept).
fn update(lua: &Lua, engine: &Engine, store: &Store, pcall: &LuaFunction, args: &Args) -> Result<LuaMultiValue, Flow> {
    let shared = shared_arg(engine, args, 0, "update")?;
    let key = key_of(args.get(1))?;
    let LuaValue::Function(callback) = args.get(2) else { return raise("Third argument to SharedTable::Update should be a function.") };
    if shared.is_frozen() {
        return raise(FROZEN);
    }
    let old = read(lua, engine, store, &shared, &key)?;
    // (through pcall: an error the callback raises comes back as its value, not mlua's text of it)
    let mut results = pcall.call::<LuaMultiValue>((callback.clone(), old))?.into_iter();
    let succeeded = matches!(results.next(), Some(LuaValue::Boolean(true)));
    let first = results.next().unwrap_or(LuaValue::Nil);
    if !succeeded {
        return Ok(values([LuaValue::Boolean(true), first, LuaValue::Integer(0)]));
    }
    // (the result is converted above update's arguments)
    write(lua, engine, store, &shared, key, &first, args.len() + 1)?;
    Ok(values([LuaValue::Boolean(false)]))
}

/// SharedTable.increment: adds `delta` to the number at `key` and returns the old one.
fn increment(engine: &Engine, args: &Args) -> Result<LuaMultiValue, Flow> {
    let shared = shared_arg(engine, args, 0, "increment")?;
    let key = key_of(args.get(1))?;
    if shared.is_frozen() {
        return raise(FROZEN);
    }
    let delta = match args.get(2) {
        LuaValue::Integer(n) => *n as f64,
        LuaValue::Number(n) => *n,
        _ => return raise("Variant cast failed"),
    };
    let old = match shared.entries.borrow().get(&key) {
        None => return raise(format!("Unable to increment the value for key \"{}\" because it does not exist in the SharedTable.", key_text(&key))),
        Some(Stored::Number(n)) => *n,
        Some(_) => return raise(format!("Unable to increment the value for key \"{}\" because it is not a number value.", key_text(&key))),
    };
    let priced = key.clone();
    let (_, copied) = shared.entries.borrow_mut().set(key, Some(Stored::Number(old + delta)));
    // (what Studio spent on it, its write included: cost.rs; and the nodes copied from a shallow copy, as a write's)
    charge(engine, cost::SHARED_INCREMENT + copied_price(&priced, copied));
    Ok(values([LuaValue::Number(old)]))
}

const UPDATE_WRAPPER: &str = r#"
local Update = ...
return function(...)
	local Failed, Problem, Level = Update(...)
	if Failed then
		error(Problem, if Level == 1 then 2 else 0)
	end
end
"#;

/// Registers the SharedTable type and its library (types.rs).
pub fn register(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let handles = lua.create_table()?;
    let weak = lua.create_table()?;
    weak.raw_set("__mode", "v")?;
    handles.set_metatable(Some(weak))?;
    let store = Rc::new(Store {
        handles,
        pool: lua.create_table()?,
        released: Rc::default(),
        free: RefCell::default(),
        next: Cell::new(0),
        tables: RefCell::default(),
        tables_kept: Cell::new(1024),
    });

    let (e, s) = (engine.clone(), store.clone());
    let index = wrapped(lua, engine, move |lua, args| {
        let args = Args::new(args);
        let shared = this(&args)?;
        let key = key_of(args.get(1))?;
        Ok(values([read(lua, &e, &s, &shared, &key)?]))
    })?;
    let (e, s) = (engine.clone(), store.clone());
    let newindex = wrapped(lua, engine, move |lua, args| {
        let args = Args::new(args);
        let shared = this(&args)?;
        let key = key_of(args.get(1))?;
        if shared.is_frozen() {
            return raise(FROZEN);
        }
        write(lua, &e, &s, &shared, key, args.get(2), 3)?;
        Ok(none())
    })?;
    let (e, s) = (engine.clone(), store.clone());
    let iter = lua.create_function(move |lua, handle: LuaAnyUserData| {
        let entries = handle.borrow::<LSharedTable>()?.0.entries.borrow().ordered();
        let mut tally = Tally::default();
        for (key, _) in &entries {
            tally.count(key);
        }
        charge(&e, cost::SHARED_LOOP + tally.strings as f64 * cost::SHARED_LOOP_STRING_ENTRY + tally.integers as f64 * cost::SHARED_LOOP_INTEGER_ENTRY);
        let mut entries = entries.into_iter();
        let (e, s) = (e.clone(), s.clone());
        lua.create_function_mut(move |lua, _: LuaMultiValue| match entries.next() {
            Some((key, value)) => Ok((key_value(lua, &key)?, load(lua, &e, &s, &value)?)),
            None => Ok((LuaValue::Nil, LuaValue::Nil)),
        })
    })?;
    let tostring = lua.create_function(|_, handle: LuaAnyUserData| Ok(format!("SharedTable: 0x{:016x}", handle.to_pointer() as usize)))?;
    lua.register_userdata_type::<LSharedTable>(|registry| {
        registry.add_meta_field("__type", "SharedTable");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__iter", iter);
        registry.add_meta_field("__tostring", tostring);
    })?;
    lock_metatable(lua, &lua.create_any_userdata(LSharedTable(Shared::new(Entries::default(), false)))?)?;

    let library = lua.create_table()?;
    let (e, s) = (engine.clone(), store.clone());
    library.set(
        "new",
        wrapped(lua, engine, move |lua, args| {
            let args = Args::new(args);
            let shared = match args.get(0) {
                _ if args.is_empty() => s.table(Entries::default(), false),
                LuaValue::Table(table) => {
                    // (charged for what it walked, also when the table fails partway)
                    let mut conversion = Conversion::new(args.len());
                    // (the table given counts against the guard as a value, as a written one does in `stored`)
                    if let Err(message) = conversion.made.add(0) {
                        return raise(message);
                    }
                    let shared = converted(lua, &e, &s, table, &mut conversion);
                    charge(&e, conversion.tally.converted_price());
                    shared?
                }
                _ => return raise_at_caller(format!("invalid argument #1 to 'new' (table expected, got {})", arg_type(&e, &args, 0))),
            };
            Ok(values([LuaValue::UserData(s.handle(lua, &shared)?)]))
        })?,
    )?;
    for (name, frozen) in [("clone", false), ("cloneAndFreeze", true)] {
        let (e, s) = (engine.clone(), store.clone());
        library.set(
            name,
            wrapped(lua, engine, move |lua, args| {
                let args = Args::new(args);
                let shared = shared_arg(&e, &args, 0, name)?;
                let deep = deep_arg(&e, &args, name)?;
                let mut tally = Tally::default();
                let copied = copy(&s, &shared, deep, frozen, &mut tally);
                // (a shallow copy shares the entries, and costs Studio the same at any size: only the call)
                charge(
                    &e,
                    tally.strings as f64 * cost::SHARED_DEEP_STRING_ENTRY + tally.integers as f64 * cost::SHARED_DEEP_INTEGER_ENTRY + tally.tables as f64 * cost::SHARED_DEEP_TABLE,
                );
                Ok(values([LuaValue::UserData(s.handle(lua, &copied)?)]))
            })?,
        )?;
    }
    let e = engine.clone();
    library.set("isFrozen", wrapped(lua, engine, move |_, args| Ok(values([LuaValue::Boolean(shared_arg(&e, &Args::new(args), 0, "isFrozen")?.is_frozen())])))?)?;
    let e = engine.clone();
    library.set(
        "size",
        wrapped(lua, engine, move |_, args| Ok(values([LuaValue::Number(shared_arg(&e, &Args::new(args), 0, "size")?.entries.borrow().len() as f64)])))?,
    )?;
    let e = engine.clone();
    library.set(
        "clear",
        wrapped(lua, engine, move |_, args| {
            let shared = shared_arg(&e, &Args::new(args), 0, "clear")?;
            if shared.is_frozen() {
                return raise(FROZEN);
            }
            // (taken out first: what it held may let go of a pool slot)
            let cleared = shared.entries.take();
            release(cleared);
            Ok(none())
        })?,
    )?;
    let e = engine.clone();
    library.set("increment", wrapped(lua, engine, move |_, args| increment(&e, &Args::new(args)))?)?;
    let (e, s) = (engine.clone(), store.clone());
    let pcall: LuaFunction = lua.globals().get("pcall")?;
    let checked = lua.create_function(move |lua, args: LuaMultiValue| match update(lua, &e, &s, &pcall, &Args::new(args)) {
        Ok(results) => Ok(results),
        Err(Flow::Error(error)) => {
            Ok(values([LuaValue::Boolean(true), LuaValue::String(lua.create_string(&error.message)?), LuaValue::Integer(error.level.into())]))
        }
        Err(Flow::Yield) => Err(LuaError::runtime("SharedTable.update cannot yield")),
        Err(Flow::Fail(error)) => Err(error),
    })?;
    let update_fn: LuaFunction = lua.load(UPDATE_WRAPPER).set_name("=ree.sharedtable").call(checked)?;
    library.set("update", update_fn)?;
    library.set_readonly(true);
    engine.lua.borrow_mut().libraries.insert("SharedTable", library);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rustc_hash::FxHashSet;

    use super::*;

    /// The order Roblox walks string keys, worked out from the keys alone (measured: [`TrieKey`] for strings): sorted by their
    /// slot level by level, a level's lone keys before its groups. The trie must walk in this order after any writes.
    fn string_order(keys: Vec<&[u8]>) -> Vec<&[u8]> {
        fn group_order<'a>(group: &mut [(u64, &'a [u8])], level: u32, order: &mut Vec<&'a [u8]>) {
            if group.len() <= 1 || level * 5 >= 64 {
                group.sort_by(|a, b| a.1.cmp(b.1));
                order.extend(group.iter().map(|(_, key)| *key));
                return;
            }
            let slot = |hash: u64| (hash >> (5 * level)) & 31;
            group.sort_by_key(|(hash, _)| slot(*hash));
            let mut groups = Vec::new();
            let mut start = 0;
            while start < group.len() {
                let at = slot(group[start].0);
                let end = start + group[start..].iter().take_while(|(hash, _)| slot(*hash) == at).count();
                if end - start == 1 {
                    order.push(group[start].1);
                } else {
                    groups.push(start..end);
                }
                start = end;
            }
            for range in groups {
                group_order(&mut group[range], level + 1, order);
            }
        }
        let mut hashed: Vec<(u64, &[u8])> = keys.into_iter().map(|key| (fnv1a(key), key)).collect();
        let mut order = Vec::new();
        group_order(&mut hashed, 0, &mut order);
        order
    }

    fn number(stored: Option<&Stored>) -> Option<f64> {
        match stored {
            Some(Stored::Number(n)) => Some(*n),
            _ => None,
        }
    }

    fn checked(entries: &Entries, numbers: &BTreeMap<u32, f64>, strings: &BTreeMap<Vec<u8>, f64>) {
        assert_eq!((entries.numbers.len, entries.strings.len), (numbers.len(), strings.len()));
        for (n, value) in numbers {
            assert_eq!(number(entries.get(&Key::Number(*n))), Some(*value));
        }
        for (s, value) in strings {
            assert_eq!(number(entries.get(&Key::String(s.as_slice().into()))), Some(*value));
        }
        let ordered = entries.ordered();
        let walked_numbers: Vec<u32> = ordered.iter().filter_map(|(key, _)| if let Key::Number(n) = key { Some(*n) } else { None }).collect();
        let walked_strings: Vec<&[u8]> = ordered.iter().filter_map(|(key, _)| if let Key::String(s) = key { Some(&**s) } else { None }).collect();
        assert_eq!(walked_numbers, numbers.keys().copied().collect::<Vec<_>>());
        assert_eq!(walked_strings, string_order(strings.keys().map(Vec::as_slice).collect()));
    }

    /// The trie holds what was written and walks in Roblox's order (the integers ascending, the strings as the keys alone give
    /// it), after any order of writes, overwrites and removals.
    #[test]
    fn the_trie_walks_in_robloxs_order_after_any_writes() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut entries = Entries::default();
        let (mut numbers, mut strings) = (BTreeMap::new(), BTreeMap::new());
        for step in 0..20_000u32 {
            let roll = next();
            let value = f64::from(step);
            let remove = roll % 3 == 0;
            if roll & 16 == 0 {
                let key = format!("k{}", (roll >> 8) % 3000).into_bytes();
                if remove {
                    entries.set(Key::String(key.as_slice().into()), None);
                    strings.remove(&key);
                } else {
                    entries.set(Key::String(key.as_slice().into()), Some(Stored::Number(value)));
                    strings.insert(key, value);
                }
            } else {
                // (small integers, and ones spread over the whole range)
                let key = if roll & 32 == 0 { ((roll >> 8) % 3000) as u32 } else { (roll >> 32) as u32 };
                if remove {
                    entries.set(Key::Number(key), None);
                    numbers.remove(&key);
                } else {
                    entries.set(Key::Number(key), Some(Stored::Number(value)));
                    numbers.insert(key, value);
                }
            }
            if step % 2500 == 0 {
                checked(&entries, &numbers, &strings);
            }
        }
        checked(&entries, &numbers, &strings);
        // emptied one key at a time, it ends as it began
        for key in numbers.keys().copied().collect::<Vec<_>>() {
            entries.set(Key::Number(key), None);
        }
        for key in strings.keys().cloned().collect::<Vec<_>>() {
            entries.set(Key::String(key.as_slice().into()), None);
        }
        assert!(entries.numbers.root.is_none() && entries.strings.root.is_none() && entries.len() == 0);
    }

    /// Keys whose bits are all equal share a list past the last level, by their order; when one is left, it moves back up.
    #[test]
    fn keys_with_equal_bits_share_a_list_and_leave_it_canonically() {
        #[derive(Clone, PartialEq, Debug)]
        struct Tiny(u8, u8);
        impl TrieKey for Tiny {
            const IN_ORDER: bool = false;
            fn bits(&self) -> u64 {
                u64::from(self.0)
            }
            fn slot(bits: u64, level: u32) -> Option<u32> {
                match level {
                    0 => Some(bits as u32 & 31),
                    1 => Some((bits >> 5) as u32 & 7),
                    _ => None,
                }
            }
            fn order(&self, other: &Self) -> Ordering {
                self.1.cmp(&other.1)
            }
        }
        let mut trie = Trie::default();
        for key in [Tiny(1, 5), Tiny(33, 0), Tiny(1, 9), Tiny(1, 3)] {
            trie.insert(key, Stored::Boolean(true), &mut 0);
        }
        let walked = |trie: &Trie<Tiny>| {
            let mut out = Vec::new();
            trie.walk(&mut out);
            out.into_iter().map(|(key, _)| key).collect::<Vec<_>>()
        };
        // (1 and 33 share slot 1 at the first level; 33 has the second level's slot 1 alone, the ones share slot 0 and then a list)
        assert_eq!(walked(&trie), [Tiny(33, 0), Tiny(1, 3), Tiny(1, 5), Tiny(1, 9)]);
        assert!(trie.get(&Tiny(1, 9)).is_some() && trie.get(&Tiny(1, 4)).is_none());
        // (a key the list does not hold takes nothing out of it)
        let bits = Tiny(1, 4).bits();
        assert!(remove(trie.root.as_mut().unwrap(), &Tiny(1, 4), bits, 0, &mut 0).is_none() && trie.len == 4);
        assert_eq!(walked(&trie), [Tiny(33, 0), Tiny(1, 3), Tiny(1, 5), Tiny(1, 9)]);
        trie.remove(&Tiny(1, 3), &mut 0);
        trie.remove(&Tiny(1, 9), &mut 0);
        assert_eq!(walked(&trie), [Tiny(1, 5), Tiny(33, 0)]);
        trie.remove(&Tiny(33, 0), &mut 0);
        let root = trie.root.as_ref().unwrap();
        assert!(root.nodes.is_empty() && root.entries.len() == 1 && root.entry_slots == 1 << 1);
    }

    fn nodes<K>(trie: &Trie<K>, out: &mut Vec<*const ()>) {
        let mut pending: Vec<&Rc<Node<K>>> = trie.root.iter().collect();
        while let Some(node) = pending.pop() {
            out.push(Rc::as_ptr(node).cast());
            pending.extend(&node.nodes);
        }
    }

    fn all_nodes(entries: &Entries) -> Vec<*const ()> {
        let mut out = Vec::new();
        nodes(&entries.numbers, &mut out);
        nodes(&entries.strings, &mut out);
        out
    }

    /// A shallow copy shares every node of its table's trie; a write to it then copies only the nodes on the key's path, and
    /// the table it was made from keeps its own entries.
    #[test]
    fn a_copy_shares_the_trie_and_a_write_copies_only_its_path() {
        let mut entries = Entries::default();
        for n in 0..10_000u32 {
            entries.set(Key::Number(n), Some(Stored::Number(f64::from(n))));
            entries.set(Key::String(format!("k{n}").into_bytes().into()), Some(Stored::Number(f64::from(n))));
        }
        let original = Shared::new(entries, false);
        let copied = Shared::new(original.entries.borrow().clone(), true);
        assert_eq!(all_nodes(&original.entries.borrow()), all_nodes(&copied.entries.borrow()));
        let (_, first) = copied.entries.borrow_mut().set(Key::Number(0), Some(Stored::Boolean(true)));
        let (_, again) = copied.entries.borrow_mut().set(Key::Number(1), Some(Stored::Boolean(true)));
        let (_, removed) = copied.entries.borrow_mut().set(Key::String(b"k0"[..].into()), None);
        // (each write counts the shared nodes it copied: none the second time down the same path)
        assert!(first >= 2 && again == 0 && removed >= 1, "{first} {again} {removed}");
        let shared: FxHashSet<*const ()> = all_nodes(&original.entries.borrow()).into_iter().collect();
        let fresh = all_nodes(&copied.entries.borrow()).into_iter().filter(|node| !shared.contains(node)).count();
        // (at most one node a level on each of the two paths: seven levels for integers, thirteen for strings)
        // (a removal may fold a subnode back into its parent: no more fresh nodes than the copies counted)
        assert!((2..=20).contains(&fresh) && first <= fresh && fresh <= first + removed, "{fresh} nodes copied");
        assert_eq!(number(original.entries.borrow().get(&Key::Number(0))), Some(0.0));
        assert_eq!(number(original.entries.borrow().get(&Key::String(b"k0"[..].into()))), Some(0.0));
        assert!(matches!(copied.entries.borrow().get(&Key::Number(0)), Some(Stored::Boolean(true))));
        assert_eq!(number(original.entries.borrow().get(&Key::Number(1))), Some(1.0));
        assert!(copied.entries.borrow().get(&Key::String(b"k0"[..].into())).is_none());
        assert_eq!((original.entries.borrow().len(), copied.entries.borrow().len()), (20_000, 19_999));
        assert_eq!(number(copied.entries.borrow().get(&Key::Number(2))), Some(2.0));
        assert!(copied.is_frozen() && !original.is_frozen());
    }

    fn store(lua: &Lua) -> Store {
        Store {
            handles: lua.create_table().unwrap(),
            pool: lua.create_table().unwrap(),
            released: Rc::default(),
            free: RefCell::default(),
            next: Cell::new(0),
            tables: RefCell::default(),
            tables_kept: Cell::new(1024),
        }
    }

    /// A chain of 300,000 SharedTables, each holding the next, is deep-copied and let go of in a loop: a call nested once a link
    /// would overflow the thread's stack.
    #[test]
    fn a_long_chain_is_copied_and_let_go_of_in_a_loop() {
        let lua = Lua::new();
        let store = store(&lua);
        let link: Rc<[u8]> = b"next"[..].into();
        let mut head = store.table(Entries::default(), false);
        for _ in 0..300_000 {
            let mut entries = Entries::default();
            entries.set(Key::String(link.clone()), Some(Stored::Shared(head)));
            head = store.table(entries, false);
        }
        let mut tally = Tally::default();
        let copied = copy(&store, &head, true, true, &mut tally);
        assert_eq!((tally.tables, tally.strings), (300_000, 300_000));
        let mut length = 0;
        let mut at = copied.clone();
        loop {
            assert!(at.is_frozen());
            let next = match at.entries.borrow().get(&Key::String(link.clone())) {
                Some(Stored::Shared(next)) => next.clone(),
                _ => break,
            };
            assert!(!Rc::ptr_eq(&next, &head));
            at = next;
            length += 1;
        }
        assert_eq!(length, 300_000);
        drop(at);
        drop(copied);
        drop(head);
    }

    /// A SharedTable that holds itself stays when nothing else does, as in Roblox, until the engine's end (its store's drop)
    /// empties it, and it goes.
    #[test]
    fn the_engines_end_empties_a_table_that_holds_itself() {
        let lua = Lua::new();
        let store = store(&lua);
        let table = store.table(Entries::default(), false);
        table.entries.borrow_mut().set(Key::String(b"self"[..].into()), Some(Stored::Shared(table.clone())));
        table.entries.borrow_mut().set(Key::String(b"bytes"[..].into()), Some(Stored::Buffer(vec![0; 1 << 20].into())));
        let weak = Rc::downgrade(&table);
        drop(table);
        assert!(weak.upgrade().is_some());
        drop(store);
        assert!(weak.upgrade().is_none());
    }

    /// A conversion counts the bytes it makes, strings, string keys and buffers, against ree's guard on a copy, and raises the
    /// guard's error past it (the count of values: `shared_table_conversions_stop_at_rees_guard_on_a_copy`).
    #[test]
    fn a_conversion_counts_its_bytes_against_rees_guard() {
        let engine = crate::Engine::new(crate::Options::default()).expect("engine");
        let lua = Lua::new();
        crate::lua::start(&lua, &engine).expect("start");
        let store = store(&lua);
        let convert = |table: &LuaTable, bytes: u64| {
            let mut conversion = Conversion::new(1);
            conversion.made.bytes = bytes;
            converted(&lua, &engine, &store, table, &mut conversion).map(|_| ())
        };
        let message = bindable::too_large(bindable::COPY_BYTES, "bytes");
        let fails = |result: Result<(), Flow>| matches!(result, Err(Flow::Error(error)) if error.message == message);
        let key: LuaTable = lua.load("return { abcd = true }").eval().unwrap();
        let text: LuaTable = lua.load("return { 'abcd' }").eval().unwrap();
        let bytes: LuaTable = lua.load("return { buffer.create(4) }").eval().unwrap();
        for table in [&key, &text, &bytes] {
            assert!(convert(table, bindable::COPY_BYTES - 4).is_ok());
            assert!(fails(convert(table, bindable::COPY_BYTES - 3)));
        }
    }

    /// The same at a run's end: once the engine's VM closes and the engine is torn down (as an Emulator's drop does), a
    /// SharedTable a script left holding itself is gone.
    #[test]
    fn a_runs_end_lets_go_of_a_table_that_holds_itself() {
        let engine = crate::Engine::new(crate::Options::default()).expect("engine");
        let lua = Lua::new();
        crate::lua::start(&lua, &engine).expect("start");
        let script = {
            let mut state = engine.state.borrow_mut();
            let class = engine.reflection.class_id("ModuleScript").expect("ModuleScript");
            crate::instance::create_named(&mut state, &engine.reflection, class, "Test")
        };
        let env = crate::lua::modules::environment(&lua, &engine, script).expect("environment");
        let source = "local S = SharedTable.new() S.self = S S.bytes = buffer.create(1048576) return S";
        let handle: LuaAnyUserData = lua.load(source).set_environment(env).eval().expect("script");
        let weak = Rc::downgrade(&handle.borrow::<LSharedTable>().expect("a SharedTable").0);
        drop(handle);
        lua.gc_collect().expect("collect");
        lua.gc_collect().expect("collect");
        assert!(weak.upgrade().is_some(), "a SharedTable that holds itself stays for the session");
        drop(lua);
        engine.teardown();
        assert!(weak.upgrade().is_none(), "the run's end let go of it");
    }
}
