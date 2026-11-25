//! SharedTable: Roblox's table for sharing data between Actors. It is a userdata here as in Roblox (`type` is "userdata",
//! `typeof` "SharedTable"), so the table functions refuse it and the VM names it in its errors. Measured in Studio 0.748, a
//! playing client:
//!
//! - Keys are strings, or integers from 0 to 2^32 - 1 (a float of an integer value is that integer; "1" and 1 are two keys).
//!   Any other key raises at the script's line, on a read as on a write: "SharedTable keys can only be strings or numbers."
//!   for a key of another type, "Numeric SharedTable keys must be nonnegative integers less than 2^32" for another number.
//! - Values: booleans, numbers, strings, vectors and Roblox's value types are kept as they are ([`KEPT`]), a buffer as a copy
//!   (and a new copy on every read), a SharedTable as itself, an InstanceHandle as a new handle of its instance (or of none),
//!   a new one on every read, `==` to the one written (Studio 0.748, tools/parity/probes/instance_handle.luau). A table becomes a new SharedTable (each place it appears, its
//!   tables too, its metatable ignored; a cycle raises "Cannot convert a cyclic table to a SharedTable"). Instances,
//!   functions and the engine's other objects raise "<Name>s can not be stored in a SharedTable"; threads, proxies, Enum,
//!   Enums and RotationCurveKeys are dropped (the key reads nil). nil removes the key.
//! - A table is converted on the Lua stack above the call's own values (new's arguments, a write's object, key and value,
//!   update's arguments and its callback's result): a table inside another sits two slots above it (its key below it), in an
//!   array as in a dictionary, and one that would sit above slot 300 raises "Max lua stack size reached", without a position,
//!   once it is known not to close a cycle (a cycle that closes there raises the cyclic message). So `new(t)` takes tables
//!   nested 150 deep and a write 149 ([`STACK_LIMIT`], measured with arrays and dictionaries; the outermost table is never
//!   refused). SharedTables already made are kept as themselves and add no depth. As every other copy of a script's table, a
//!   conversion counts what it makes against ree's guard on one copy and stops past it ([`stored`]), the empty slots of the
//!   tables it walks too ([`converted`]).
//! - A write checks the key, then converts a table value (its errors come first), then checks whether the table is frozen
//!   ("SharedTable is frozen"), then any other value ([`write`]). Only the key's errors and the library's argument errors carry
//!   the script's position.
//! - `for k, v in S` walks the entries as they were when the loop started: the integer keys ascending, then the string keys in
//!   the order of Roblox's hash trie ([`string_order`]), keys whose 64-bit hashes are equal in their list's order ([`find`]).
//!   The loop keeps the entries it started on until the collector takes its function, after its last step too ([`Walk`]).
//! - `#`, `next`, `pairs`, `rawget`, `setmetatable` and the `table` library raise as on any userdata; the metatable is locked;
//!   `S:size()` raises "attempt to call missing method 'size' of SharedTable", and tostring is "SharedTable: 0x" and 16 hex
//!   digits.
//! - A SharedTable is a new userdata each time it reaches a script (a read, a loop's value, `new`, `clone`, the registry's Get,
//!   what a BindableEvent or BindableFunction delivers), `==` to every other of the same SharedTable and to no other, while
//!   rawequal, tostring and a table keyed by one tell them apart ([`handle`]).
//! - The library (a read-only table, its keys added in an order whose loop gives a client's: cloneAndFreeze, clear, clone,
//!   isFrozen, size, increment, update, new): `new([table])`, `clone(S [, deep])` and `cloneAndFreeze(S [, deep])` (a shallow
//!   copy shares the SharedTables it holds, a deep one copies them, frozen too for cloneAndFreeze; a clone of a frozen table is
//!   not frozen; given two arguments or more, `deep` is checked before the table), `isFrozen`, `size`, `clear`,
//!   `increment(S, key, delta)` (returns the old value; a delta left out raises "Failed to get value for insertion into
//!   SharedTable" after the key's check and before the frozen flag's; a key it does not find is left in the table with no
//!   value ([`Stored::Nil`]) before it raises, but only in a table no engine value was made of: a client marks a SharedTable
//!   for good once it stores it in a SharedTable, the registry, a bindable's copy, SetAttribute, print or warn, deep-copies it
//!   or updates it, and a marked one gets no such key ([`Shared::mark`], measured); its messages quote the key up to its first
//!   NUL, a message at most [`MESSAGE_LIMIT`] bytes) and `update(S, key, callback)` (the callback gets the old value and
//!   returns the new one, which is converted at once, so one that cannot be stored raises then; the callback is called again
//!   with the value then current whenever a write, increment, clear or update of the table completed while it ran, even one
//!   that changed nothing (nil written to a key the table does not hold, a clear of an empty table), not a call that raised, a
//!   read, a clone or a write to another table ([`Shared::version`]); it runs one C call deep, where it cannot yield, so
//!   updates nest up to the VM's limit of 200 C calls, less those the thread holds (199 deep in a client's script), before
//!   "C stack overflow", and an error it raises passes through update as raised). update and increment are one C function
//!   each, as a client's are ([`update_c`], [`increment_c`]: `debug.info` gives `[C]` and their names, a callback's level 2 is
//!   update itself, and a callback's error reaches an `xpcall` handler and the Output's stack at the raise, the callback's line
//!   first; measured, Studio 0.748, a playing client). Called with no argument at all, each
//!   but `new` (which makes an empty table) raises "missing argument #1 to '<name>' (SharedTable expected)" at the script's
//!   line. Their messages are below, as measured.
//!
//! Where the contents live: in the engine, as Roblox keeps them in its own (numbers, booleans, vectors, strings and buffers as
//! data, the value types the engine has values for as those, a SharedTable it holds as a shared reference), and each userdata
//! a script holds is a handle of its own. A SharedTable holds none of mlua's reference slots (lua/refs.rs). The entries are a
//! persistent hash trie, which Roblox's timings and orders fit ([`Trie`]): a shallow copy shares it, and a write copies only
//! the nodes on its path that a copy shares, so a shallow copy takes the same time at any size, and a write right after one
//! pays for the path it copies (measured: 0.47 to 1.16 us for the first write after cloning 10 to 10,000 string-keyed entries,
//! where an overwrite takes 0.08). Keys whose 64-bit hashes are all equal share a list past the trie's last level, kept as
//! Roblox keeps it (measured with real FNV-1a collisions; a client kept 8,192 keys of one hash and read them back, no cap up to
//! there, and ree sets none): the newest written first, searched from the front; a key added goes first and a removal keeps the
//! others' order; an overwrite or increment in a list the table alone holds stays in place, while a write in a list a shallow
//! copy (or a loop, [`Walk`]) shares copies the list and moves the key written first (a removal there keeps the others'
//! order); `update` always moves its key first, as it holds a copy of the table while it runs; a deep copy keeps the order,
//! `new(t)` adds t's keys in its `pairs` order (so they walk the other way), and a list left with one key moves back up. Not
//! matched: once a client has marked a SharedTable ([`Shared::mark`], which ree tracks for increment), every overwrite or
//! increment of a key in a list moves it first, as in a list a copy shares (measured, iteration probes held, trigger and
//! sticky); ree's stays in place. A deep copy shares the nodes that hold no SharedTable with the table it copied
//! ([`retarget`]), so a write to either right after it pays for its path and moves a key of a list first, as after a shallow
//! copy (a client's top-level deep copy does the same, measured). The value types the engine has no value for (Ray, Axes,
//! TweenInfo, the keypoints, ...) wait in one engine table, a slot each, freed once no SharedTable holds them; a copy goes in
//! and a read hands out a new object of the one kept ([`fresh`]), as a client's do. A
//! SharedTable that holds itself is kept, while its handles are not: measured, a client collected the handles of 300
//! SharedTables that held themselves and their 300 MB of buffers stayed (plain ones gave theirs back). Here it is kept until
//! the engine's end, which empties every SharedTable still alive ([`Store`]'s drop). However long a chain of SharedTables a
//! script builds (each holding the next), letting go of it and deep-copying it walk the chain in a loop, never a call nested
//! once a link (Studio deep-copied chains 1,000 long; ree's deep copy stops only at its guard on a copy, [`copy`]).
//!
//! What their calls cost the work clock: cost.rs [`crate::cost::SHARED_NEW_STRING_ENTRY`] and the prices after it, by the
//! entries they handle, the nodes and lists a write copies or rebuilds and the list entries it compares, the bytes of a key
//! (hashed once a call and again for each further call of an update's callback, copied in when a write adds it), of a string or
//! buffer copied in or made for a script on a read or a loop's step, the object a read or a loop's step makes ([`load`]), an
//! integer key a read finds, the empty slots a conversion walks and the entries `clear` frees. A read of a 16-byte string now
//! costs what a client's does ([`cost::SHARED_MADE_STRING`]). Not matched yet: a read by a 16-byte key took a client 0.059 to
//! 0.080 us (ree charges 0.03); a loop's step over 16-byte string values 0.025 to 0.029 more than over numbers (ree 0.003); an
//! update with a trivial callback 0.70 in one probe and 0.38 to 0.43 in another (ree 0.32); increment 0.07 to 0.21 in three
//! probes against an older probe's 0.37 to 0.43 ([`cost::SHARED_INCREMENT`]; ree charges 0.39); one key added and taken out
//! again and again 0.22 to 0.31 us a pair (ree 0.62, an add and a removal); reads of one list of colliding
//! keys again and again about a third of ree's price (a client's cache: 2.0 to 2.4 us for the oldest of 1,024, ree 7.6; read
//! once in each of 200 tables 8.4 to 8.7, matched); SharedTableRegistry's GetSharedTable 0.17 to 0.19 us and SetSharedTable
//! 0.35 to 0.37 (ree charges a method call, 0.085); and from 65,488 bytes a client takes longer over a key than its hash
//! ([`cost::SHARED_KEY_BYTE`]).

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::{Rc, Weak};

use mlua::{ffi, prelude::*};
use rustc_hash::FxHashMap;

use super::bindable::{self, STACK_FULL, STACK_LIMIT};
use super::{Args, Flow, convert, lock_metatable, none, raise, raise_at_caller, raw, typeof_value, values, wrapped};
use super::types::LTweenInfo;
use crate::{Engine, cost, value::Value};
use lune_roblox::datatypes::types as lune;

/// A SharedTable as a script holds it: a handle to its contents, a new one each time the table reaches a script ([`handle`]).
pub struct LSharedTable(Rc<Shared>);

/// A SharedTable: whether it is frozen, whether an engine value was made of it, its entries, whose nodes the copies made of it
/// share, and how often they changed. Its identity is this allocation.
struct Shared {
    frozen: Cell<bool>,
    /// Set for good once the engine makes a value of the table, as a client marks one ([`Shared::mark`]): from then on a failed
    /// increment leaves no key behind ([`increment`]).
    marked: Cell<bool>,
    /// Bumped by every write, increment, clear and update's store that completes, whatever it changed: a client's update calls
    /// its callback again after each of those, also nil or a dropped value written to a key the table does not hold, a clear of
    /// an empty table and a nested update storing nothing there, but not after one that raised (measured, Studio 0.748, a
    /// playing client: two calls each; a failed increment, a write of a function, a nested update with an unstorable result,
    /// one call). Update calls its callback again when it moved while the callback ran ([`update_commit`])
    version: Cell<u64>,
    entries: RefCell<Entries>,
}

impl Shared {
    fn new(entries: Entries, frozen: bool) -> Rc<Shared> {
        Rc::new(Shared { frozen: Cell::new(frozen), marked: Cell::new(false), version: Cell::new(0), entries: RefCell::new(entries) })
    }

    fn is_frozen(&self) -> bool {
        self.frozen.get()
    }

    /// Marks the table for good: a client marks a SharedTable once it makes an engine value of it, after which a failed
    /// increment of a key the table does not hold leaves nothing (measured, Studio 0.748, a playing client, two runs of each
    /// probe). It marks the table stored in a SharedTable (a write, also one that then raises on the frozen flag, new's nested
    /// tables, update's result, a table a write converts, a table holding it that a write or new converts up to where the
    /// conversion fails), set in or made by SharedTableRegistry (Set before it checks the name), sent by a BindableEvent or
    /// BindableFunction (at top level or in a table, Invoke's results too), given to SetAttribute, print or warn, the source of a
    /// deep copy and the copies of the tables it holds, and the table of an update whose checks passed. Nothing else marks one: a
    /// read, a loop, size, tostring, a shallow clone or a deep copy's top table (also of a marked table), JSONEncode, a property
    /// cast, a key refused before the value, an update refused by its checks.
    fn mark(&self) {
        self.marked.set(true);
    }

    /// Marks a change of the entries.
    fn changed(&self) {
        self.version.set(self.version.get() + 1);
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        release(std::mem::take(self.entries.get_mut()));
    }
}

/// Lets go of `entries` one SharedTable at a time: a SharedTable they were the last to hold is emptied in turn here, not
/// dropped inside this drop, so a chain of any length (each holding the next) never nests a drop once a link. The entries it
/// freed: those of the nodes no other table shared ([`Trie::drain`]), what `clear` is charged for.
fn release(entries: Entries) -> usize {
    let mut freed = 0;
    let mut pending = vec![entries];
    while let Some(entries) = pending.pop() {
        entries.drain(&mut |value| {
            freed += 1;
            if let Stored::Shared(shared) = value
                && let Some(mut shared) = Rc::into_inner(shared)
            {
                pending.push(std::mem::take(shared.entries.get_mut()));
            }
        });
    }
    freed
}

#[derive(Clone, Default)]
struct Entries {
    numbers: Trie<u32>,
    strings: Trie<Rc<[u8]>>,
}

/// A value as a SharedTable holds it.
#[derive(Clone)]
enum Stored {
    /// A key with no value: what increment leaves at a key it did not find before it raises, in a table no engine value was made
    /// of ([`Shared::mark`]; measured, Studio 0.748, a playing client: size counts it, a loop gives it as (key, nil) in its key's
    /// place, clones, deep copies and what a BindableEvent delivers keep it, as does marking the table afterwards, a read and
    /// update's callback get nil, another increment says it does not exist; writing nil, a dropped value or update's nil takes it
    /// out, as clear does; not a change for update)
    Nil,
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

/// A key as a call names it: an integer, or a string's bytes borrowed from the script's own string with their hash, worked
/// out once a call ([`key_of`]). Nothing is copied to look a key up: a write that adds the key copies its bytes in
/// ([`TrieKey::make`]).
enum Probe {
    Number(u32),
    String(LuaBorrowedBytes, u64),
}

/// A key to look for: an integer, or a string's bytes and their hash.
#[derive(Clone, Copy)]
enum KeyRef<'a> {
    Number(u32),
    String(&'a [u8], u64),
}

impl Probe {
    fn key(&self) -> KeyRef<'_> {
        match self {
            Probe::Number(n) => KeyRef::Number(*n),
            Probe::String(bytes, bits) => KeyRef::String(&bytes[..], *bits),
        }
    }

    fn is_string(&self) -> bool {
        matches!(self, Probe::String(..))
    }

    /// The bytes of a string key, hashed once by the call
    fn len(&self) -> usize {
        if let Probe::String(bytes, _) = self { bytes.len() } else { 0 }
    }

    /// What hashing the key cost (cost.rs [`cost::SHARED_KEY_BYTE`]), charged by the call that named it, also when it then raises
    fn price(&self) -> f64 {
        self.len() as f64 * cost::SHARED_KEY_BYTE
    }
}

impl Entries {
    /// The value at `key`, the search counted in `work`.
    fn get(&self, key: KeyRef, work: &mut Work) -> Option<&Stored> {
        self.locate(key, work).map(|(value, _)| value)
    }

    /// The value at `key` and its place in a list past the trie's last level when it is in one ([`Trie::locate`]), the search
    /// counted in `work`.
    fn locate(&self, key: KeyRef, work: &mut Work) -> Option<(&Stored, Option<usize>)> {
        match key {
            KeyRef::Number(n) => self.numbers.locate(&n, u64::from(n), work),
            KeyRef::String(bytes, bits) => self.strings.locate(bytes, bits, work),
        }
    }

    /// Puts `value` over the entry at `key` that [`Entries::locate`] found, `found` its place in a list: the list is not searched
    /// again. What it did is counted in `work` as [`Entries::set`] counts it.
    fn replace(&mut self, key: KeyRef, value: Stored, found: Option<usize>, work: &mut Work) -> Option<Stored> {
        match key {
            KeyRef::Number(n) => self.numbers.insert_at(&n, u64::from(n), value, found, work),
            KeyRef::String(bytes, bits) => self.strings.insert_at(bytes, bits, value, found, work),
        }
    }

    /// Sets the entry at `key` (None removes it): the value it replaces, for the caller to drop once it has let go of the
    /// entries, what it did counted in `work` ([`Trie::insert`]).
    fn set(&mut self, key: KeyRef, value: Option<Stored>, work: &mut Work) -> Option<Stored> {
        match (key, value) {
            (KeyRef::Number(n), Some(value)) => self.numbers.insert(&n, u64::from(n), value, work),
            (KeyRef::String(bytes, bits), Some(value)) => self.strings.insert(bytes, bits, value, work),
            (KeyRef::Number(n), None) => self.numbers.remove(&n, u64::from(n), work),
            (KeyRef::String(bytes, bits), None) => self.strings.remove(bytes, bits, work),
        }
    }

    fn len(&self) -> usize {
        self.numbers.len + self.strings.len
    }

    /// How many entries hold a SharedTable (each node keeps the count of those below it: [`Node::tables`]).
    fn tables(&self) -> usize {
        self.numbers.tables() + self.strings.tables()
    }

    /// Every value of the nodes no other copy shares ([`Trie::drain`]).
    fn drain(self, out: &mut impl FnMut(Stored)) {
        self.numbers.drain(out);
        self.strings.drain(out);
    }
}

/// The keys of a [`Trie`]: the bits it branches on, five a level.
trait TrieKey: Clone {
    /// The key as a call names it: an integer, or a string's bytes, borrowed
    type Ref: ?Sized;
    /// Whether a node walks its slots in order, entries and subnodes mixed (integers, which so come ascending), or its
    /// entries first and then its subnodes (strings, as Roblox walks them)
    const IN_ORDER: bool;
    fn view(&self) -> &Self::Ref;
    /// The key a write adds, made from the one the call names: the one copy of a string's bytes
    fn make(key: &Self::Ref) -> Self;
    /// The bytes of a key, which hashing or copying it reads
    fn bytes(key: &Self::Ref) -> usize;
    /// A held key's bits: a string's hashed again (only a slot split asks, [`insert_into`])
    fn bits(&self) -> u64;
    /// The slot of `bits` at `level`; None past the last level, where the keys left have equal bits
    fn slot(bits: u64, level: u32) -> Option<u32>;
    /// Whether two keys are equal, and the bytes comparing them read
    fn compare(a: &Self::Ref, b: &Self::Ref) -> (bool, usize);
}

/// Integer keys branch on their own bits from the highest (two, then six levels of five), so walking the slots in order walks
/// the integers ascending. Two never share every slot, so an integer list never forms.
impl TrieKey for u32 {
    type Ref = u32;
    const IN_ORDER: bool = true;

    fn view(&self) -> &u32 {
        self
    }

    fn make(key: &u32) -> u32 {
        *key
    }

    fn bytes(_: &u32) -> usize {
        0
    }

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

    fn compare(a: &u32, b: &u32) -> (bool, usize) {
        (a == b, 0)
    }
}

/// String keys branch on the 64-bit FNV-1a hash of their bytes, five bits a level from the lowest; a node's entries come
/// before its subnodes. Measured: 299 random keys, 40 keys sharing a prefix, and keys after deletes and re-adds all came out in
/// this order, whatever order they were added in. Keys whose hashes are equal share a list past the last level ([`find`]).
impl TrieKey for Rc<[u8]> {
    type Ref = [u8];
    const IN_ORDER: bool = false;

    fn view(&self) -> &[u8] {
        self
    }

    fn make(key: &[u8]) -> Self {
        Rc::from(key)
    }

    fn bytes(key: &[u8]) -> usize {
        key.len()
    }

    fn bits(&self) -> u64 {
        fnv1a(self)
    }

    fn slot(bits: u64, level: u32) -> Option<u32> {
        (level * 5 < 64).then(|| (bits >> (5 * level)) as u32 & 31)
    }

    /// (by blocks of 64 bytes, as fast as a memcmp, up to the first that differs: what Roblox's byte comparison of a list's keys
    /// reads, measured 0.03 to 0.05 ns a byte with keys sharing a 64 KB prefix)
    fn compare(a: &[u8], b: &[u8]) -> (bool, usize) {
        if a.len() != b.len() {
            return (false, 0);
        }
        let mut read = 0;
        for (x, y) in a.chunks(64).zip(b.chunks(64)) {
            read += x.len();
            if x != y {
                return (false, read);
            }
        }
        (true, read)
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// What a trie operation did beyond its call's own price, priced by [`Work::price`].
#[derive(Default)]
struct Work {
    /// Plain nodes copied because another trie shared them
    copied: usize,
    /// Entries of shared lists copied
    list_copied: usize,
    /// List entries compared, and the bytes comparing them read
    scanned: usize,
    compared: usize,
    /// Entries of lists a key joined or left
    rebuilt: usize,
    /// Bytes of held keys a slot split hashed again
    rehashed: usize,
    /// Bytes of keys added, copied in
    added: usize,
}

impl Work {
    /// What it cost (cost.rs: [`cost::SHARED_COPIED_STRING_NODE`], [`cost::SHARED_LIST_SCANNED`] and after, the key bytes'
    /// prices), a string key's or an integer key's.
    fn price(&self, string: bool) -> f64 {
        self.copied as f64 * if string { cost::SHARED_COPIED_STRING_NODE } else { cost::SHARED_COPIED_INTEGER_NODE }
            + self.list_copied as f64 * cost::SHARED_LIST_COPIED
            + self.scanned as f64 * cost::SHARED_LIST_SCANNED
            + self.compared as f64 * cost::SHARED_COMPARED_BYTE
            + self.rebuilt as f64 * cost::SHARED_LIST_REBUILT
            + self.rehashed as f64 * cost::SHARED_KEY_BYTE
            + self.added as f64 * cost::SHARED_KEY_ADDED_BYTE
    }
}

/// A persistent hash trie, as Roblox keeps a SharedTable's entries: a copy shares every node, and a write copies only the nodes
/// on its path that another copy shares. It is canonical (a CHAMP): a key sits at the shallowest level where its slot is its
/// own, whatever order keys came and went in, so the order it walks in is a function of its keys, but for the lists of keys
/// whose bits are all equal ([`find`]).
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

/// A node of a [`Trie`]: 32 slots, each empty, an entry or a subnode. Past the last level its entries have no slots: a list,
/// the newest written first.
#[derive(Clone)]
struct Node<K> {
    entry_slots: u32,
    node_slots: u32,
    /// The entries at and below this node that hold a SharedTable, so a deep copy walks only the nodes that lead to one
    /// ([`retarget`]). It changes only through [`node_mut`], which copies a node another trie shares first, so a shared node
    /// keeps the count of what it holds.
    tables: usize,
    entries: Vec<(K, Stored)>,
    nodes: Vec<Rc<Node<K>>>,
}

impl<K> Default for Node<K> {
    fn default() -> Self {
        Node { entry_slots: 0, node_slots: 0, tables: 0, entries: Vec::new(), nodes: Vec::new() }
    }
}

/// 1 for a SharedTable, the values a node's [`Node::tables`] counts.
fn held(value: &Stored) -> usize {
    usize::from(matches!(value, Stored::Shared(_)))
}

/// Where the slot `bit` goes among the slots `slots` holds.
fn below(slots: u32, bit: u32) -> usize {
    (slots & (bit - 1)).count_ones() as usize
}

impl<K: TrieKey> Trie<K> {
    /// The value at `key`, whose bits are `bits`: a slot's key compared once, a list searched from the front (counted).
    #[cfg(test)]
    fn get(&self, key: &K::Ref, bits: u64, work: &mut Work) -> Option<&Stored> {
        self.locate(key, bits, work).map(|(value, _)| value)
    }

    /// The value at `key`, whose bits are `bits`, and its place in a list past the last level when it is in one (None at a
    /// slot): a slot's key compared once, a list searched from the front (counted).
    fn locate(&self, key: &K::Ref, bits: u64, work: &mut Work) -> Option<(&Stored, Option<usize>)> {
        let mut node = self.root.as_deref()?;
        let mut level = 0;
        loop {
            let Some(slot) = K::slot(bits, level) else {
                return find(&node.entries, key, work).map(|at| (&node.entries[at].1, Some(at)));
            };
            let bit = 1 << slot;
            if node.entry_slots & bit != 0 {
                let (other, value) = &node.entries[below(node.entry_slots, bit)];
                return K::compare(other.view(), key).0.then_some((value, None));
            }
            if node.node_slots & bit == 0 {
                return None;
            }
            node = &node.nodes[below(node.node_slots, bit)];
            level += 1;
        }
    }

    /// Puts `value` at `key`, whose bits are `bits`: the value it replaces. `work` counts the nodes on its path that another
    /// trie shared, which it copied first, a list's search and rebuild, and the key's bytes when it adds it (the one copy).
    fn insert(&mut self, key: &K::Ref, bits: u64, value: Stored, work: &mut Work) -> Option<Stored> {
        self.insert_at(key, bits, value, None, work)
    }

    /// [`Trie::insert`], `found` the key's place in a list when [`Trie::locate`] found it there, so the list is not searched
    /// again.
    fn insert_at(&mut self, key: &K::Ref, bits: u64, value: Stored, found: Option<usize>, work: &mut Work) -> Option<Stored> {
        let old = insert(self.root.get_or_insert_with(Rc::default), key, bits, value, 0, found, work);
        if old.is_none() {
            self.len += 1;
        }
        old
    }

    /// Takes `key` out: its value, counting what it did as [`Trie::insert`] does. Nothing is copied for a key the trie does not
    /// hold: it is looked for first (counted), and a key in a list is taken out at the place found, the list searched once.
    fn remove(&mut self, key: &K::Ref, bits: u64, work: &mut Work) -> Option<Stored> {
        let found = self.locate(key, bits, work)?.1;
        let old = remove(self.root.as_mut()?, key, bits, 0, found, work)?;
        self.len -= 1;
        if self.len == 0 {
            self.root = None;
        }
        Some(old)
    }

    /// How many entries hold a SharedTable.
    fn tables(&self) -> usize {
        self.root.as_ref().map_or(0, |root| root.tables)
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

/// The node, to change: copied first when another trie shares it (counted in `work`, a list by its entries), and whether it
/// was. `list`: whether it is a list past the last level.
fn node_mut<'a, K: TrieKey>(node: &'a mut Rc<Node<K>>, list: bool, work: &mut Work) -> (&'a mut Node<K>, bool) {
    let shared = Rc::strong_count(node) > 1;
    if shared {
        if list {
            work.list_copied += node.entries.len();
        } else {
            work.copied += 1;
        }
    }
    (Rc::make_mut(node), shared)
}

/// Where `key` is in a list past the last level, searched from the front (the newest written first), each entry compared
/// counted: Roblox keeps keys whose 64-bit hashes are equal in such a list and compares them in turn (measured: 8,192 keys of
/// one hash kept, the oldest read in 38 us and the newest in 0.33).
fn find<K: TrieKey>(entries: &[(K, Stored)], key: &K::Ref, work: &mut Work) -> Option<usize> {
    entries.iter().position(|(other, _)| {
        work.scanned += 1;
        let (equal, read) = K::compare(other.view(), key);
        work.compared += read;
        equal
    })
}

/// [`Trie::insert_at`] at a node `level` deep.
fn insert<K: TrieKey>(node: &mut Rc<Node<K>>, key: &K::Ref, bits: u64, value: Stored, level: u32, found: Option<usize>, work: &mut Work) -> Option<Stored> {
    let added = held(&value);
    let (node, shared) = node_mut(node, K::slot(bits, level).is_none(), work);
    let old = insert_into(node, shared, key, bits, value, level, found, work);
    // (one entry changed below this node, the same change at each level of its path; a split moves an entry within the node's
    // own subtree, which keeps its count)
    node.tables = node.tables + added - old.as_ref().map_or(0, held);
    old
}

#[allow(clippy::too_many_arguments)]
fn insert_into<K: TrieKey>(node: &mut Node<K>, shared: bool, key: &K::Ref, bits: u64, value: Stored, level: u32, found: Option<usize>, work: &mut Work) -> Option<Stored> {
    let Some(slot) = K::slot(bits, level) else {
        // (a list, as Roblox keeps one (measured): a key written in a list a copy shared moves first in the copy, the others in
        // their order; in a list the table alone holds it stays in place; a key added goes first, rebuilding the list; a place
        // the call already found is not searched for again)
        return match found.or_else(|| find(&node.entries, key, work)) {
            Some(at) if shared => {
                let (kept, old) = node.entries.remove(at);
                node.entries.insert(0, (kept, value));
                Some(old)
            }
            Some(at) => Some(std::mem::replace(&mut node.entries[at].1, value)),
            None => {
                if !shared {
                    work.rebuilt += node.entries.len();
                }
                work.added += K::bytes(key);
                node.entries.insert(0, (K::make(key), value));
                None
            }
        };
    };
    let bit = 1 << slot;
    if node.entry_slots & bit != 0 {
        let at = below(node.entry_slots, bit);
        if K::compare(node.entries[at].0.view(), key).0 {
            return Some(std::mem::replace(&mut node.entries[at].1, value));
        }
        // (two keys in one slot: fresh nodes for both, as many levels down as their slots stay the same; the key held is hashed
        // again, once, as Roblox's is (measured: a 3-byte key added and taken out beside a 1 MB key in its slot 793 us))
        let (kept, kept_value) = node.entries.remove(at);
        node.entry_slots &= !bit;
        work.rehashed += K::bytes(kept.view());
        work.added += K::bytes(key);
        let kept_bits = kept.bits();
        let child = split((kept, kept_bits, kept_value), (K::make(key), bits, value), level + 1);
        node.nodes.insert(below(node.node_slots, bit), child);
        node.node_slots |= bit;
        return None;
    }
    if node.node_slots & bit != 0 {
        let at = below(node.node_slots, bit);
        return insert(&mut node.nodes[at], key, bits, value, level + 1, found, work);
    }
    work.added += K::bytes(key);
    node.entries.insert(below(node.entry_slots, bit), (K::make(key), value));
    node.entry_slots |= bit;
    None
}

/// Two keys that met in one slot, `kept` held and `new` written, with their bits: fresh nodes from `level` down, as many as
/// their slots stay equal, ending in a slot each, or past the last level in a list, the new key first.
fn split<K: TrieKey>(kept: (K, u64, Stored), new: (K, u64, Stored), level: u32) -> Rc<Node<K>> {
    let mut node = Node { tables: held(&kept.2) + held(&new.2), ..Node::default() };
    match (K::slot(kept.1, level), K::slot(new.1, level)) {
        (Some(a), Some(b)) if a == b => {
            node.nodes.push(split(kept, new, level + 1));
            node.node_slots = 1 << a;
        }
        (Some(a), Some(b)) => {
            let (first, second) = if a < b { (kept, new) } else { (new, kept) };
            node.entry_slots = (1 << a) | (1 << b);
            node.entries = vec![(first.0, first.2), (second.0, second.2)];
        }
        _ => node.entries = vec![(new.0, new.2), (kept.0, kept.2)],
    }
    Rc::new(node)
}

/// [`Trie::remove`] at a node `level` deep: the key's value, or None (and nothing taken) when the node does not hold it.
/// `found`: the key's place in a list, already found. A subnode left with one entry and no subnode gives the entry back to its
/// parent, which keeps the trie canonical.
fn remove<K: TrieKey>(node: &mut Rc<Node<K>>, key: &K::Ref, bits: u64, level: u32, found: Option<usize>, work: &mut Work) -> Option<Stored> {
    let (node, shared) = node_mut(node, K::slot(bits, level).is_none(), work);
    let old = remove_from(node, shared, key, bits, level, found, work)?;
    // (an entry a subnode gives back to this node stays in its subtree: only the key taken out changes the count)
    node.tables -= held(&old);
    Some(old)
}

fn remove_from<K: TrieKey>(node: &mut Node<K>, shared: bool, key: &K::Ref, bits: u64, level: u32, found: Option<usize>, work: &mut Work) -> Option<Stored> {
    let Some(slot) = K::slot(bits, level) else {
        // (the others keep their order, in a list the table holds, rebuilt, as in one a copy shared, copied without the key:
        // measured)
        let at = found.or_else(|| find(&node.entries, key, work))?;
        if !shared {
            work.rebuilt += node.entries.len();
        }
        return Some(node.entries.remove(at).1);
    };
    let bit = 1 << slot;
    if node.entry_slots & bit != 0 {
        let at = below(node.entry_slots, bit);
        if !K::compare(node.entries[at].0.view(), key).0 {
            return None;
        }
        node.entry_slots &= !bit;
        return Some(node.entries.remove(at).1);
    }
    if node.node_slots & bit == 0 {
        return None;
    }
    let at = below(node.node_slots, bit);
    let old = remove(&mut node.nodes[at], key, bits, level + 1, found, work)?;
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

/// A walk over a trie's entries in loop order, one at a time: the nodes from the root down to the one it is in, with where it
/// is in each, so its own memory goes by the trie's depth, not its size. The root it holds pins the version of the trie it
/// started on: shared with the table, which adds nothing, until the table changes, after which a write copies its path first
/// (as a write after a shallow copy does) and the old version stays alive for the walk. So it walks the entries as they were
/// when it started.
struct Cursor<K> {
    stack: Vec<(Rc<Node<K>>, usize)>,
}

impl<K: TrieKey> Cursor<K> {
    fn new(trie: &Trie<K>) -> Self {
        Cursor { stack: trie.root.iter().map(|root| (root.clone(), 0)).collect() }
    }
}

/// A loop under way: its walks over the integer and string keys, and the entries it started on, which it keeps for as long as
/// the loop's function lives, after its last step too, as a client's loop does (measured: right after a loop that ran to its
/// end, an overwrite, an increment and an update in a list each moved the key first, as after a shallow copy). The collector
/// takes the function once nothing holds it.
struct Walk {
    numbers: Cursor<u32>,
    strings: Cursor<Rc<[u8]>>,
    _started: Entries,
}

impl<K: TrieKey> Iterator for Cursor<K> {
    type Item = (K, Stored);

    fn next(&mut self) -> Option<(K, Stored)> {
        loop {
            let level = self.stack.len().checked_sub(1)? as u32;
            let (node, at) = self.stack.last_mut()?;
            let child = if K::IN_ORDER && K::slot(0, level).is_some() {
                // (the slots in order, entries and subnodes mixed)
                let mut child = None;
                while *at < 32 && child.is_none() {
                    let bit = 1u32 << *at;
                    *at += 1;
                    if node.entry_slots & bit != 0 {
                        return Some(node.entries[below(node.entry_slots, bit)].clone());
                    }
                    if node.node_slots & bit != 0 {
                        child = Some(node.nodes[below(node.node_slots, bit)].clone());
                    }
                }
                child
            } else if *at < node.entries.len() {
                *at += 1;
                return Some(node.entries[*at - 1].clone());
            } else {
                *at += 1;
                node.nodes.get(*at - 1 - node.entries.len()).cloned()
            };
            match child {
                Some(child) => self.stack.push((child, 0)),
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}

/// A pool slot of the [`Store`]: its value is dropped from the pool once the last SharedTable holding it lets go (at the
/// next SharedTable read or value pooled, [`Store::drain`]; a drop may come from the collector, where no Lua may run).
struct PoolSlot {
    id: u32,
    released: Rc<RefCell<Vec<u32>>>,
    /// What a read making a new object of it costs ([`load`]: cost.rs [`cost::SHARED_MADE_VALUE`] or
    /// [`cost::SHARED_MADE_WIDE_VALUE`]; a loop's step pays [`cost::SHARED_LOOP_MADE_VALUE`] for any)
    made: f64,
}

impl Drop for PoolSlot {
    fn drop(&mut self) {
        if let Ok(mut released) = self.released.try_borrow_mut() {
            released.push(self.id);
        }
    }
}

/// The engine's part of every SharedTable: the pool of the values the engine has no value of its own for, and every
/// SharedTable made, to empty those still alive at the engine's end.
struct Store {
    pool: LuaTable,
    released: Rc<RefCell<Vec<u32>>>,
    free: RefCell<Vec<u32>>,
    next: Cell<u32>,
    tables: RefCell<Vec<Weak<Shared>>>,
    /// How long `tables` may grow before those let go of are taken out
    tables_kept: Cell<usize>,
    /// The prelude's CopyProxy, once a read needed it ([`fresh`])
    copy_proxy: RefCell<Option<LuaFunction>>,
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

    /// Empties the pool slots let go of since the last time: the values no SharedTable holds any more.
    fn drain(&self) -> LuaResult<()> {
        if self.released.borrow().is_empty() {
            return Ok(());
        }
        // (what was let go: taken out before any Lua runs, which may collect and let go of more)
        let released = std::mem::take(&mut *self.released.borrow_mut());
        for id in released {
            self.pool.raw_set(id, LuaValue::Nil)?;
            self.free.borrow_mut().push(id);
        }
        Ok(())
    }

    /// Pools `value`, a read of which costs `made`.
    fn put(&self, value: LuaValue, made: f64) -> LuaResult<Rc<PoolSlot>> {
        self.drain()?;
        let reused = self.free.borrow_mut().pop();
        let id = reused.unwrap_or_else(|| {
            self.next.set(self.next.get() + 1);
            self.next.get()
        });
        self.pool.raw_set(id, value)?;
        Ok(Rc::new(PoolSlot { id, released: self.released.clone(), made }))
    }
}

/// A new handle of `shared` for a script. Roblox makes a new userdata each time a SharedTable reaches a script, `==` to every
/// other of the same table (the `__eq` in [`register`]) and to no other (measured in a playing client, Studio 0.748: `S.t`
/// read twice, a loop's value against a read, a value written and read back and the registry's Get called twice are each `==`
/// but not rawequal, their tostring differs, and a table keyed by one is not found by another; two SharedTables, a clone and
/// its original among them, are not `==`). What a BindableEvent or BindableFunction delivers is a new handle too, at top level
/// and inside a table, for Fire and for Invoke's arguments and results (measured: rawequal false, tostring different, `==`
/// true; lua/bindable.rs).
fn handle(lua: &Lua, shared: &Rc<Shared>) -> LuaResult<LuaAnyUserData> {
    lua.create_any_userdata(LSharedTable(shared.clone()))
}

/// A new handle of the SharedTable `held` is one of: what SharedTableRegistry:GetSharedTable returns (services.luau, stubs.rs
/// `SharedTableHandle`) and what a bindable delivers (lua/bindable.rs `rebuild`).
pub fn fresh_handle(lua: &Lua, held: &LuaAnyUserData) -> LuaResult<LuaAnyUserData> {
    let shared = held.borrow::<LSharedTable>()?.0.clone();
    handle(lua, &shared)
}

/// Marks the SharedTable `value` is a handle of, if it is one ([`Shared::mark`]): the engine making a value of it elsewhere (the
/// registry, services.luau; a bindable's copy, lua/bindable.rs; SetAttribute, lua/instance.rs; print and warn, prelude.luau).
pub fn mark_value(value: &LuaValue) {
    if let LuaValue::UserData(ud) = value
        && let Ok(held) = ud.borrow::<LSharedTable>()
    {
        held.0.mark();
    }
}

/// Entries a call handled, for its price: by their key's kind, the SharedTables a deep clone copied, and what a conversion
/// spent by the bytes and slots it handled (in seconds).
#[derive(Default)]
struct Tally {
    strings: u64,
    integers: u64,
    tables: u64,
    seconds: f64,
}

impl Tally {
    fn count(&mut self, string: bool) {
        if string {
            self.strings += 1;
        } else {
            self.integers += 1;
        }
    }

    fn count_entries(&mut self, entries: &Entries) {
        self.integers += entries.numbers.len as u64;
        self.strings += entries.strings.len as u64;
    }

    /// What converting the counted entries costs (SharedTable.new from a table, or a table written to a key).
    fn converted_price(&self) -> f64 {
        self.strings as f64 * cost::SHARED_NEW_STRING_ENTRY + self.integers as f64 * cost::SHARED_NEW_INTEGER_ENTRY + self.seconds
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

/// What a write costs beyond the VM's call to it and its key's hash, as timed in a client (cost.rs [`cost::SHARED_OVERWRITE_STRING`]
/// and after): an overwrite, or a key added or taken out (`changed`), by the key's kind, and the work it did ([`Work::price`]).
fn write_price(string: bool, changed: bool, work: &Work) -> f64 {
    let own = match (string, changed) {
        (true, false) => cost::SHARED_OVERWRITE_STRING,
        (true, true) => cost::SHARED_CHANGE_STRING,
        (false, false) => cost::SHARED_OVERWRITE_INTEGER,
        (false, true) => cost::SHARED_CHANGE_INTEGER,
    };
    own + work.price(string)
}

/// What copying in a string's or a buffer's (`buffer`) `bytes` bytes costs: one a write stores (or update's result) by Studio's
/// two tiers (cost.rs [`cost::SHARED_COPIED_BYTE`]; dearer from [`cost::SHARED_LARGE_STRING`] bytes for a string and
/// [`cost::SHARED_LARGE_BUFFER`] for a buffer), one inside a converted table (`converted`) at the conversion's price a byte.
fn copied_in(bytes: usize, buffer: bool, converted: bool) -> f64 {
    let large = if buffer { cost::SHARED_LARGE_BUFFER } else { cost::SHARED_LARGE_STRING };
    if converted {
        bytes as f64 * cost::SHARED_CONVERTED_BYTE
    } else if bytes >= large {
        cost::SHARED_LARGE_COPY_BLOCK + bytes as f64 * cost::SHARED_LARGE_COPY_BYTE
    } else {
        bytes as f64 * cost::SHARED_COPIED_BYTE
    }
}

/// What making a string of `bytes` bytes for a script costs: the VM's own price a byte (cost.rs [`cost::LUAU_STRING_BYTE`], in
/// ns), which it leaves out of the strings an engine call makes.
fn made_string(bytes: usize) -> f64 {
    bytes as f64 * cost::LUAU_STRING_BYTE * 1e-9
}

/// SharedTableRegistry's name for a buffer (services.luau `RegistryName`): its contents as a string, charged before it is copied
/// as a string the script built ([`made_string`], cost.rs LUAU_STRING_BYTE a byte). A Get keeps a table under every new name
/// for the session, so a name made of a buffer's bytes is paid for as the script's own string of them would be.
pub fn buffer_name(lua: &Lua, engine: &Engine, value: &LuaValue) -> LuaResult<LuaString> {
    let LuaValue::Buffer(buffer) = value else { return Err(LuaError::runtime("BufferRegistryName: a buffer expected")) };
    charge(engine, made_string(buffer.len()));
    lua.create_string(buffer.to_vec())
}

/// What a string of `bytes` bytes a read makes for a script costs (a read, update's old value): [`made_string`]'s price and a
/// client's base for a string a read makes (cost.rs [`cost::SHARED_MADE_STRING`]), and from [`cost::SHARED_LARGE_STRING`]
/// bytes, where a client's reads step as its writes do, a client's (cost.rs [`cost::SHARED_LOADED_STRING`]). A loop's keys and
/// values do not step (measured), so a loop prices its strings by [`made_string`].
fn read_string(bytes: usize) -> f64 {
    if bytes >= cost::SHARED_LARGE_STRING {
        cost::SHARED_LOADED_STRING + bytes as f64 * cost::SHARED_LOADED_STRING_BYTE
    } else {
        cost::SHARED_MADE_STRING + made_string(bytes)
    }
}

/// What making a buffer of `bytes` bytes for a script costs (cost.rs [`cost::SHARED_LOADED_BUFFER`]).
fn made_buffer(bytes: usize) -> f64 {
    let block = if bytes >= cost::SHARED_LOADED_BUFFER_FROM { cost::SHARED_LOADED_BUFFER } else { 0.0 };
    block + bytes as f64 * cost::SHARED_LOADED_BUFFER_BYTE
}

const KEY_TYPE: &str = "SharedTable keys can only be strings or numbers.";
const KEY_NUMBER: &str = "Numeric SharedTable keys must be nonnegative integers less than 2^32";
const FROZEN: &str = "SharedTable is frozen";
const CYCLIC: &str = "Cannot convert a cyclic table to a SharedTable";

/// The longest message increment raises (measured: with a missing key of 1 MB its message came out 1,000,000 bytes, cut
/// inside the key, so it ends with no closing quote).
const MESSAGE_LIMIT: usize = 1_000_000;

/// The value types a SharedTable keeps as they are (measured, each read back as itself). Native vectors, booleans, numbers and
/// strings are kept too.
const KEPT: &[&str] = &[
    "Axes", "BrickColor", "CFrame", "Color3", "ColorSequence", "ColorSequenceKeypoint", "EnumItem", "Faces", "Font", "InstanceHandle", "NumberRange",
    "NumberSequence", "NumberSequenceKeypoint", "Path2DControlPoint", "PhysicalProperties", "Ray", "Rect", "Region3", "Region3int16",
    "SecurityCapabilities", "TweenInfo", "UDim", "UDim2", "Vector2", "Vector2int16", "Vector3int16",
];

/// The value types a SharedTable drops without a word (measured: the key reads nil after the write).
const DROPPED: &[&str] = &["Enum", "Enums", "RotationCurveKey", "userdata"];

/// The pooled value types whose object a read makes costs a client more (cost.rs [`cost::SHARED_MADE_WIDE_VALUE`]).
const WIDE: &[&str] = &["TweenInfo", "Region3", "PhysicalProperties", "Path2DControlPoint"];

/// The key a call names, its string's bytes hashed once, where they are ([`Probe`]).
fn key_of(value: &LuaValue) -> Result<Probe, Flow> {
    let number = match value {
        LuaValue::String(s) => {
            let bytes = s.as_bytes();
            let bits = fnv1a(&bytes);
            return Ok(Probe::String(bytes, bits));
        }
        LuaValue::Integer(n) => *n as f64,
        LuaValue::Number(n) => *n,
        _ => return raise_at_caller(KEY_TYPE),
    };
    if (0.0..4294967296.0).contains(&number) && number.fract() == 0.0 { Ok(Probe::Number(number as u32)) } else { raise_at_caller(KEY_NUMBER) }
}

/// An increment message about `key`: the key as Roblox quotes it, cut at its first NUL (measured: "a\0b" is quoted "a"), and
/// the whole message cut at [`MESSAGE_LIMIT`] bytes. The key's bytes are kept as they are, UTF-8 or not (measured: "a\255b"
/// quoted in 3 bytes), so the message is raised as bytes ([`increment_c`]), not as a script error's text.
fn increment_message(key: &Probe, reason: &str) -> Vec<u8> {
    let mut message = b"Unable to increment the value for key \"".to_vec();
    match key {
        Probe::Number(n) => message.extend_from_slice(n.to_string().as_bytes()),
        Probe::String(bytes, _) => {
            // (the key's own bytes, UTF-8 or not: measured, "a\xffb" is quoted as it is)
            let quoted = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
            message.extend_from_slice(&quoted[..quoted.len().min(MESSAGE_LIMIT)]);
        }
    }
    message.extend_from_slice(format!("\" because {reason}.").as_bytes());
    message.truncate(MESSAGE_LIMIT);
    message
}

fn refused<T>(name: &str) -> Result<T, Flow> {
    raise(format!("{name}s can not be stored in a SharedTable"))
}

/// How far above a table one inside it sits on the Lua stack while it is converted: its key and itself, in an array as in a
/// dictionary (measured: `new` takes arrays and dictionaries alike nested 150 deep).
const NESTED: usize = 2;

/// How a value is being converted: the stack slot a table found sits at ([`converted`]), the tables open around it (to find a
/// cycle), the entries converted so far and what they cost (for the price), and what the conversion has made against ree's
/// guard on one copy.
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
/// A string's or a buffer's bytes are copied in ([`copied_in`], priced in the conversion's tally): the value a write stores at
/// a write's price, one inside a converted table at the conversion's.
fn stored(lua: &Lua, engine: &Engine, store: &Store, value: &LuaValue, conversion: &mut Conversion) -> Result<Option<Stored>, Flow> {
    let bytes = match value {
        LuaValue::String(s) => s.as_bytes().len(),
        LuaValue::Buffer(buffer) => buffer.len(),
        _ => 0,
    };
    if let Err(message) = conversion.made.add(bytes) {
        return raise(message);
    }
    if matches!(value, LuaValue::String(_) | LuaValue::Buffer(_)) {
        conversion.tally.seconds += copied_in(bytes, matches!(value, LuaValue::Buffer(_)), !conversion.open.is_empty());
    }
    Ok(Some(match value {
        LuaValue::Nil | LuaValue::Thread(_) | LuaValue::LightUserData(_) | LuaValue::Error(_) | LuaValue::Other(_) => return Ok(None),
        LuaValue::Boolean(b) => Stored::Boolean(*b),
        LuaValue::Integer(n) => Stored::Number(*n as f64),
        LuaValue::Number(n) => Stored::Number(*n),
        LuaValue::Vector(v) => Stored::Vector(v.x(), v.y(), v.z()),
        LuaValue::String(s) => Stored::String(Rc::from(&*s.as_bytes())),
        LuaValue::Buffer(buffer) => Stored::Buffer(buffer.to_vec().into()),
        LuaValue::Function(_) => return refused("Function"),
        // (a SharedTable stored in another, given or converted, is marked: [`Shared::mark`])
        LuaValue::Table(table) => {
            let shared = converted(lua, engine, store, table, conversion)?;
            shared.mark();
            Stored::Shared(shared)
        }
        LuaValue::UserData(ud) => {
            if let Ok(handle) = ud.borrow::<LSharedTable>() {
                handle.0.mark();
                return Ok(Some(Stored::Shared(handle.0.clone())));
            }
            if raw::instance_of_ud(ud).is_some() {
                return refused("Instance");
            }
            let name = typeof_value(engine, value);
            match name.as_str() {
                name if KEPT.contains(&name) => match convert::from_lua(value) {
                    // (a copy goes in, not the script's object: a client copies a Path2DControlPoint whenever it is stored, a
                    // write, new, a nested table and update's result alike, so changing the script's object later changes
                    // nothing stored, measured; the other pooled types cannot be changed)
                    Value::Lua(_) => {
                        let made = if WIDE.contains(&name) { cost::SHARED_MADE_WIDE_VALUE } else { cost::SHARED_MADE_VALUE };
                        Stored::Pooled(store.put(fresh(lua, engine, store, value.clone())?, made)?)
                    }
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

/// The array slots and hash nodes of `table` that hold nothing, which walking it visits as well (the VM's own count of them:
/// patches/luau/12-profiling.patch `lua_profiletable`).
fn empty_slots(table: &LuaTable) -> usize {
    // SAFETY: a live table, which the call holds
    match unsafe { crate::profiler::vm::table_shape(table.to_pointer() as *mut c_void) } {
        Some(shape) => shape.array.saturating_sub(shape.array_used) as usize + shape.nodes.saturating_sub(shape.nodes_used) as usize,
        None => 0,
    }
}

/// A table as a new SharedTable (its own tables converted too), its entries counted as they go (also those walked before a
/// failure), each string key hashed once and copied in. A table inside another sits [`NESTED`] slots above it; one that is
/// already open closes a cycle, and otherwise one above [`STACK_LIMIT`] is too deep (in that order, measured: a cycle closing
/// at the limit raises the cyclic message). The outermost table is never refused. Its slots, read at once, are checked against
/// ree's guard on a copy before any is walked (each counts against it, an entry or an empty slot, so a table past it raises
/// at once, the same error the walk would reach); its empty array slots and hash nodes are then walked as Roblox walks them,
/// priced (cost.rs [`cost::SHARED_EMPTY_SLOT`]) and counted against the guard as a copy's holes are, before its entries.
fn converted(lua: &Lua, engine: &Engine, store: &Store, table: &LuaTable, conversion: &mut Conversion) -> Result<Rc<Shared>, Flow> {
    let pointer = table.to_pointer();
    if conversion.open.contains(&pointer) {
        return raise(CYCLIC);
    }
    if !conversion.open.is_empty() && conversion.height > STACK_LIMIT {
        return raise(STACK_FULL);
    }
    // SAFETY: the table is held for the call (its pointer read while `table` lives)
    if let Some((array, nodes)) = unsafe { crate::profiler::vm::table_slots(table.to_pointer().cast_mut()) }
        && let Err(message) = conversion.made.check_values(u64::from(array) + u64::from(nodes))
    {
        return raise(message);
    }
    let empty = empty_slots(table);
    conversion.tally.seconds += empty as f64 * cost::SHARED_EMPTY_SLOT;
    if let Err(message) = conversion.made.add_values(empty as u64) {
        return raise(message);
    }
    conversion.open.push(pointer);
    conversion.height += NESTED;
    let mut entries = Entries::default();
    // Walk entries directly on the C stack with `lua_next` so primitive/string/buffer/thread
    // pairs avoid per-entry mlua registry refs; pause onto `key_of` + `stored` only when a value
    // is a nested table or userdata (which may recurse or call into mlua/typeof).
    let mut cursor = LuaValue::Nil;
    loop {
        let mut paused = false;
        let mut failed: Option<Flow> = None;
        let (paused_key, paused_val): (LuaValue, LuaValue) = unsafe {
            lua.exec_raw((table, cursor), |state| {
                while ffi::lua_next(state, 1) != 0 {
                    // stack: 1 = table, 2 = key, 3 = value
                    let val_type = ffi::lua_type(state, 3);
                    if val_type == ffi::LUA_TTABLE || val_type == ffi::LUA_TUSERDATA {
                        // Leave [key, value] on the stack for `exec_raw` to return and resume after.
                        ffi::lua_remove(state, 1);
                        paused = true;
                        return;
                    }
                    let key_type = ffi::lua_type(state, 2);
                    let (is_string, key_len, num_key, str_ptr, str_bits) = match key_type {
                        ffi::LUA_TNUMBER => {
                            let number = ffi::lua_tonumber(state, 2);
                            if !((0.0..4294967296.0).contains(&number) && number.fract() == 0.0) {
                                ffi::lua_settop(state, 0);
                                failed = Some(raise_at_caller::<()>(KEY_NUMBER).unwrap_err());
                                return;
                            }
                            (false, 0usize, number as u32, std::ptr::null(), 0u64)
                        }
                        ffi::LUA_TSTRING => {
                            let mut len = 0usize;
                            let ptr = ffi::lua_tolstring(state, 2, &mut len) as *const u8;
                            let bytes = if len == 0 { &[] } else { std::slice::from_raw_parts(ptr, len) };
                            (true, len, 0u32, ptr, fnv1a(bytes))
                        }
                        _ => {
                            ffi::lua_settop(state, 0);
                            failed = Some(raise_at_caller::<()>(KEY_TYPE).unwrap_err());
                            return;
                        }
                    };
                    conversion.tally.count(is_string);
                    conversion.tally.seconds += key_len as f64 * cost::SHARED_KEY_BYTE;
                    if let Err(message) = conversion.made.add_bytes(key_len) {
                        ffi::lua_settop(state, 0);
                        failed = Some(raise::<()>(message).unwrap_err());
                        return;
                    }
                    let val_bytes = if val_type == ffi::LUA_TSTRING || val_type == ffi::LUA_TBUFFER {
                        ffi::lua_objlen(state, 3) as usize
                    } else {
                        0
                    };
                    if let Err(message) = conversion.made.add(val_bytes) {
                        ffi::lua_settop(state, 0);
                        failed = Some(raise::<()>(message).unwrap_err());
                        return;
                    }
                    if val_type == ffi::LUA_TSTRING || val_type == ffi::LUA_TBUFFER {
                        conversion.tally.seconds += copied_in(val_bytes, val_type == ffi::LUA_TBUFFER, !conversion.open.is_empty());
                    }
                    let value = match val_type {
                        ffi::LUA_TNIL | ffi::LUA_TTHREAD | ffi::LUA_TLIGHTUSERDATA => None,
                        ffi::LUA_TBOOLEAN => Some(Stored::Boolean(ffi::lua_toboolean(state, 3) != 0)),
                        ffi::LUA_TNUMBER => Some(Stored::Number(ffi::lua_tonumber(state, 3))),
                        ffi::LUA_TVECTOR => {
                            let v = ffi::lua_tovector(state, 3);
                            Some(Stored::Vector(*v, *v.add(1), *v.add(2)))
                        }
                        ffi::LUA_TSTRING => {
                            let mut len = 0usize;
                            let ptr = ffi::lua_tolstring(state, 3, &mut len) as *const u8;
                            let bytes = if len == 0 { &[] } else { std::slice::from_raw_parts(ptr, len) };
                            Some(Stored::String(Rc::from(bytes)))
                        }
                        ffi::LUA_TBUFFER => {
                            let mut len = 0usize;
                            let ptr = ffi::lua_tobuffer(state, 3, &mut len) as *const u8;
                            let bytes = if len == 0 { &[] } else { std::slice::from_raw_parts(ptr, len) };
                            Some(Stored::Buffer(Rc::from(bytes)))
                        }
                        ffi::LUA_TFUNCTION => {
                            ffi::lua_settop(state, 0);
                            failed = Some(refused::<()>("Function").unwrap_err());
                            return;
                        }
                        _ => None,
                    };
                    let key_ref = if is_string {
                        let bytes = if key_len == 0 { &[] } else { std::slice::from_raw_parts(str_ptr, key_len) };
                        KeyRef::String(bytes, str_bits)
                    } else {
                        KeyRef::Number(num_key)
                    };
                    let mut work = Work::default();
                    entries.set(key_ref, value, &mut work);
                    conversion.tally.seconds += work.price(is_string);
                    ffi::lua_pop(state, 1);
                }
                ffi::lua_settop(state, 0);
            })
        }?;
        if let Some(flow) = failed {
            return Err(flow);
        }
        if !paused {
            break;
        }
        let key = key_of(&paused_key)?;
        conversion.tally.count(key.is_string());
        conversion.tally.seconds += key.price();
        if let Err(message) = conversion.made.add_bytes(key.len()) {
            return raise(message);
        }
        let value = stored(lua, engine, store, &paused_val, conversion)?;
        let mut work = Work::default();
        entries.set(key.key(), value, &mut work);
        conversion.tally.seconds += work.price(key.is_string());
        cursor = paused_key;
    }
    conversion.height -= NESTED;
    conversion.open.pop();
    Ok(store.table(entries, false))
}

/// A stored value as a script gets it (a read, a loop's step, update's old value), the objects it makes charged: a read's
/// (`read`) string by [`read_string`], a loop's by [`made_string`], a buffer by [`made_buffer`], a value type's userdata and a
/// SharedTable's new handle as a client's, by a read (cost.rs [`cost::SHARED_MADE_VALUE`] and after) or by a loop's step, which
/// pays one price for any of them ([`cost::SHARED_LOOP_MADE_VALUE`]); a number, a boolean and a vector make none.
fn load(lua: &Lua, engine: &Engine, store: &Store, stored: &Stored, read: bool) -> LuaResult<LuaValue> {
    store.drain()?;
    let made = |price: f64| if read { price } else { cost::SHARED_LOOP_MADE_VALUE };
    Ok(match stored {
        Stored::Nil => LuaValue::Nil,
        Stored::Boolean(b) => LuaValue::Boolean(*b),
        Stored::Number(n) => LuaValue::Number(*n),
        Stored::Vector(x, y, z) => LuaValue::Vector(mlua::Vector::new(*x, *y, *z)),
        Stored::String(bytes) => {
            charge(engine, if read { read_string(bytes.len()) } else { made_string(bytes.len()) });
            LuaValue::String(lua.create_string(&**bytes)?)
        }
        Stored::Buffer(bytes) => {
            charge(engine, made_buffer(bytes.len()));
            LuaValue::Buffer(lua.create_buffer(&**bytes)?)
        }
        Stored::Value(value) => {
            charge(engine, made(cost::SHARED_MADE_VALUE));
            convert::to_lua(lua, engine, value)?
        }
        Stored::Pooled(slot) => {
            charge(engine, made(slot.made));
            fresh(lua, engine, store, store.pool.raw_get(slot.id)?)?
        }
        Stored::Shared(shared) => {
            charge(engine, made(cost::SHARED_MADE_HANDLE));
            LuaValue::UserData(handle(lua, shared)?)
        }
    })
}

/// A copy of a pooled value: what goes in when one is stored ([`stored`]) and what a read hands out, a new object `==` to the
/// one written (measured, Studio 0.748, a playing client: Ray, Region3, PhysicalProperties, NumberSequenceKeypoint,
/// ColorSequenceKeypoint, TweenInfo, Faces, Axes, Path2DControlPoint, Region3int16, Vector2int16, Vector3int16 and
/// SecurityCapabilities each read back not rawequal to the value written nor to another read, by a read, a loop, update's old
/// value and a clone's read alike). The prelude copies its proxies (`CopyProxy`); another pooled type (not measured) comes
/// back as it was written. A copy is the engine's own work, which its caller prices ([`engine_work`]).
fn fresh(lua: &Lua, engine: &Engine, store: &Store, value: LuaValue) -> LuaResult<LuaValue> {
    let LuaValue::UserData(ud) = &value else { return Ok(value) };
    macro_rules! copied {
        ($(($kind:ty, $tag:ident)),*) => {$(
            if let Ok(held) = ud.borrow::<$kind>() {
                let copy = *held;
                drop(held);
                return Ok(LuaValue::UserData(convert::lune_userdata(lua, convert::LuneKind::$tag, copy)?));
            }
        )*};
    }
    copied!(
        (lune::Ray, Ray),
        (lune::Region3, Region3),
        (lune::PhysicalProperties, PhysicalProperties),
        (lune::NumberSequenceKeypoint, NumberSequenceKeypoint),
        (lune::ColorSequenceKeypoint, ColorSequenceKeypoint),
        (lune::Region3int16, Region3int16),
        (lune::Vector2int16, Vector2int16),
        (lune::Vector3int16, Vector3int16)
    );
    if ud.is::<super::types::LInstanceHandle>() {
        return super::types::copy_instance_handle(lua, ud);
    }
    if let Ok(info) = ud.borrow::<LTweenInfo>() {
        let copy = LTweenInfo {
            time: info.time.clone(),
            easing_style: info.easing_style,
            easing_direction: info.easing_direction,
            repeat_count: info.repeat_count.clone(),
            reverses: info.reverses,
            delay_time: info.delay_time.clone(),
        };
        drop(info);
        let userdata = lua.create_any_userdata(copy)?;
        engine_work(engine, || super::lock_metatable(lua, &userdata))?;
        return Ok(LuaValue::UserData(userdata));
    }
    let cached = store.copy_proxy.borrow().clone();
    let copy = match cached {
        Some(copy) => copy,
        None => {
            let copy: LuaFunction = engine.prelude().exports.get("CopyProxy")?;
            *store.copy_proxy.borrow_mut() = Some(copy.clone());
            copy
        }
    };
    engine_work(engine, || copy.call(value))
}

/// Runs `work`, a call of the engine's own into the VM (a TweenInfo's copy locking its metatable through a C call, a prelude
/// proxy's copy calling into the prelude), taking back what the VM's counters priced of it as the script's work (cost.rs
/// [`cost::luau`]). They count it when the script's own frame made the call that copies, a loop's step or update (its old
/// value and its result), and not under the prelude's wrapper a read or a write runs in; a client's copy makes no such calls, so
/// the price of what is copied is its caller's alone (measured: a loop's step over TweenInfos or Path2DControlPoints cost a
/// client what one over CFrames does, cost.rs [`cost::SHARED_LOOP_MADE_VALUE`], where the VM's count put 0.012 and 0.015 us on
/// ree's).
pub(crate) fn engine_work<T>(engine: &Engine, work: impl FnOnce() -> LuaResult<T>) -> LuaResult<T> {
    let counters = engine.state.borrow().sched.vm_counters;
    // SAFETY: the VM's own counters, alive as long as the VM, which is running this call
    let priced = || counters.map_or(0.0, |counters| cost::luau(unsafe { &*counters }));
    let before = priced();
    let result = work();
    let counted = priced() - before;
    if counted != 0.0 {
        let state = engine.state.borrow();
        state.sched.charge(-counted);
        if !state.sched.is_muted() {
            crate::boundary::add_cost(-counted);
        }
    }
    result
}

/// A deep copy under way: the SharedTables it has copied, by the original's address (a table held twice, or holding itself, is
/// copied once, as in Roblox: measured), the copies whose entries still hold the originals' tables, what it has made against
/// ree's guard on a copy, and the entries it is priced by.
struct DeepCopy<'a> {
    store: &'a Store,
    frozen: bool,
    copies: FxHashMap<*const Shared, Rc<Shared>>,
    pending: Vec<Rc<Shared>>,
    made: &'a mut bindable::Tally,
    tally: &'a mut Tally,
}

impl DeepCopy<'_> {
    /// The copy of `original`, made the first time it is reached with the original's entries (shared until its turn comes:
    /// [`copy`]). Studio copies every entry of every table a deep copy reaches, shallow clones included (measured), so each
    /// copy is priced by all its entries and the SharedTables they hold, counts a node keeps, whatever ree walks of it.
    fn copy_of(&mut self, original: &Rc<Shared>) -> Result<Rc<Shared>, String> {
        if let Some(copy) = self.copies.get(&Rc::as_ptr(original)) {
            return Ok(copy.clone());
        }
        self.made.add(0)?;
        let entries = original.entries.borrow().clone();
        self.tally.count_entries(&entries);
        self.tally.tables += entries.tables() as u64;
        let copy = self.store.table(entries, self.frozen);
        self.copies.insert(Rc::as_ptr(original), copy.clone());
        self.pending.push(copy.clone());
        Ok(copy)
    }
}

/// `node` with every SharedTable below it swapped for its copy: the node itself when none is below it ([`Node::tables`]), and
/// one copy of it however many tables share it (`done`, by the original node), so the copies of tables that shared a node share
/// its copy. What the copy makes, each slot of the node, counts against ree's guard first. A trie is at most 14 levels deep. A
/// list is copied as it is, in its order (a deep copy keeps a list's order: measured).
fn retarget<K: TrieKey>(node: &Rc<Node<K>>, deep: &mut DeepCopy, done: &mut FxHashMap<*const Node<K>, Rc<Node<K>>>) -> Result<Rc<Node<K>>, String> {
    if node.tables == 0 {
        return Ok(node.clone());
    }
    if let Some(copy) = done.get(&Rc::as_ptr(node)) {
        return Ok(copy.clone());
    }
    deep.made.add_values((node.entries.len() + node.nodes.len()) as u64)?;
    let mut copy = Node { entry_slots: node.entry_slots, node_slots: node.node_slots, tables: node.tables, entries: node.entries.clone(), nodes: Vec::with_capacity(node.nodes.len()) };
    for (_, value) in &mut copy.entries {
        if let Stored::Shared(original) = value {
            // (a copy stored in another is marked, as a client marks a deep copy's nested tables: [`Shared::mark`])
            let nested = deep.copy_of(original)?;
            nested.mark();
            *original = nested;
        }
    }
    for child in &node.nodes {
        copy.nodes.push(retarget(child, deep, done)?);
    }
    let copy = Rc::new(copy);
    done.insert(Rc::as_ptr(node), copy.clone());
    Ok(copy)
}

/// A copy of `shared`, frozen or not. A shallow one shares its entries' trie (a write to either copies only its path); a deep
/// one copies the SharedTables they hold as well ([`DeepCopy`]), walking only the nodes that lead to one and copying each such
/// node once, so it makes no more than the tables it copies hold, and stops at ree's guard on a copy past that (its error;
/// the copies made so far are emptied, so one that holds itself does not outlive the call). It walks the tables with a stack of
/// its own, so a chain of any length never nests a call once a link. `tally`: the entries priced, also when it stops; `made`:
/// what the call has made against the guard.
fn copy(store: &Store, shared: &Rc<Shared>, deep: bool, frozen: bool, tally: &mut Tally, made: &mut bindable::Tally) -> Result<Rc<Shared>, String> {
    if !deep {
        return Ok(store.table(shared.entries.borrow().clone(), frozen));
    }
    // (the source of a deep copy is marked, the top copy is not: [`Shared::mark`])
    shared.mark();
    let mut deep = DeepCopy { store, frozen, copies: FxHashMap::default(), pending: Vec::new(), made, tally };
    let (mut numbers, mut strings) = (FxHashMap::default(), FxHashMap::default());
    let copied = deep.copy_of(shared).and_then(|result| {
        while let Some(copy) = deep.pending.pop() {
            // (its entries are still the original's, shared: each node leading to a SharedTable is swapped for its copy)
            let mut entries = copy.entries.borrow().clone();
            if let Some(root) = &entries.numbers.root {
                entries.numbers.root = Some(retarget(root, &mut deep, &mut numbers)?);
            }
            if let Some(root) = &entries.strings.root {
                entries.strings.root = Some(retarget(root, &mut deep, &mut strings)?);
            }
            drop(copy.entries.replace(entries));
        }
        Ok(result)
    });
    drop((numbers, strings));
    if copied.is_err() {
        for (_, copy) in deep.copies.drain() {
            release(copy.entries.take());
        }
    }
    copied
}

/// Argument `index` (0-based) of library function `function`, which must be a SharedTable: its contents. One not passed at all
/// is Luau's missing argument, nil and other values an invalid one (measured, Studio 0.748, a playing client: `size()` raises
/// "missing argument #1 to 'size' (SharedTable expected)", `size(nil)` "invalid argument #1 to 'size' (SharedTable expected,
/// got nil)", each at the script's line).
fn shared_arg(engine: &Engine, args: &Args, index: usize, function: &str) -> Result<Rc<Shared>, Flow> {
    match args.get(index) {
        LuaValue::UserData(ud) if ud.is::<LSharedTable>() => Ok(ud.borrow::<LSharedTable>()?.0.clone()),
        _ if index >= args.len() => raise_at_caller(format!("missing argument #{} to '{function}' (SharedTable expected)", index + 1)),
        other => raise_at_caller(format!("invalid argument #{} to '{function}' (SharedTable expected, got {})", index + 1, typeof_value(engine, other))),
    }
}

/// The SharedTable a metamethod was called on.
fn this(args: &Args) -> Result<Rc<Shared>, Flow> {
    match args.get(0) {
        LuaValue::UserData(ud) => Ok(ud.borrow::<LSharedTable>()?.0.clone()),
        _ => raise("invalid SharedTable"),
    }
}

/// The optional `deep` argument of clone and cloneAndFreeze: false when not passed, else a boolean. Checked before the table
/// (measured, Studio 0.748, a playing client: `clone(5, 5)` raises "invalid argument #2 to 'clone' (boolean expected, got
/// number)", `clone(nil, nil)` argument #2's message too, `clone(5)` and `clone(5, true)` argument #1's).
fn deep_arg(engine: &Engine, args: &Args, function: &str) -> Result<bool, Flow> {
    match args.get(1) {
        _ if args.len() < 2 => Ok(false),
        LuaValue::Boolean(deep) => Ok(*deep),
        other => raise_at_caller(format!("invalid argument #2 to '{function}' (boolean expected, got {})", typeof_value(engine, other))),
    }
}

/// The value at `key` in `entries`, for a script, and what finding it cost (a list's search, an integer key that found a value:
/// cost.rs [`cost::SHARED_FOUND_INTEGER`]) charged.
fn read(lua: &Lua, engine: &Engine, store: &Store, entries: &Entries, key: &Probe) -> LuaResult<LuaValue> {
    let mut work = Work::default();
    let (value, found) = match entries.get(key.key(), &mut work) {
        Some(stored) => (load(lua, engine, store, stored, true)?, !key.is_string() && !matches!(stored, Stored::Nil)),
        None => (LuaValue::Nil, false),
    };
    charge(engine, work.price(key.is_string()) + if found { cost::SHARED_FOUND_INTEGER } else { 0.0 });
    Ok(value)
}

/// `value` as a SharedTable stores it ([`stored`]: None for no entry), charging `spent` (what the call spent before it: its
/// key's hash) with the conversion: a table is converted at `new`'s price, also for the entries walked before a conversion
/// fails. `height`: the values on the stack below it ([`Conversion::new`]).
fn convert_value(lua: &Lua, engine: &Engine, store: &Store, value: &LuaValue, height: usize, spent: f64) -> Result<Option<Stored>, Flow> {
    let mut conversion = Conversion::new(height);
    let value = stored(lua, engine, store, value, &mut conversion);
    charge(engine, spent + conversion.tally.converted_price());
    value
}

/// Sets `key` of `entries` (a table's, or update's copy of them) to `value` (converted: [`convert_value`]; the key already
/// checked, the table not frozen), priced as an overwrite or a key added or taken out ([`write_price`]): the value it replaced,
/// for the caller to drop once it has let go of the entries.
fn put(engine: &Engine, entries: &mut Entries, key: &Probe, value: Option<Stored>) -> Option<Stored> {
    let adds = value.is_some();
    let mut work = Work::default();
    let replaced = entries.set(key.key(), value, &mut work);
    charge(engine, write_price(key.is_string(), adds != replaced.is_some(), &work));
    replaced
}

/// Writes `value` at `key` (the key already checked; `spent` its hash): [`convert_value`], then [`put`], a change for update
/// whatever it stored or took out ([`Shared::version`]). A table value is converted before the frozen flag is looked at, so its
/// conversion's errors come first and the SharedTables it reaches are marked, and a SharedTable value is marked before it, while
/// any other value is checked after it (measured, Studio 0.748, a playing client, two runs: on a frozen table `{ print }` raises
/// "Functions can not be stored in a SharedTable", a cyclic table the cyclic message, `{ S }` and `S` mark S and raise "SharedTable
/// is frozen", and `print` raises "SharedTable is frozen"; a frozen table's conversion is charged as any other, not measured).
#[allow(clippy::too_many_arguments)]
fn write(lua: &Lua, engine: &Engine, store: &Store, shared: &Shared, key: &Probe, value: &LuaValue, height: usize, spent: f64) -> Result<(), Flow> {
    if shared.is_frozen() && !matches!(value, LuaValue::Table(_)) {
        mark_value(value);
        charge(engine, spent);
        return raise(FROZEN);
    }
    let value = convert_value(lua, engine, store, value, height, spent)?;
    if shared.is_frozen() {
        return raise(FROZEN);
    }
    let replaced = put(engine, &mut shared.entries.borrow_mut(), key, value);
    shared.changed();
    drop(replaced);
    Ok(())
}

/// What update's and increment's C functions reach their Rust through (their first upvalue): the engine, the SharedTables'
/// store and a table values cross in (their second upvalue, as lua/raw.rs bridges values). The Rust runs in the C function's
/// own frame, as raw.rs's does, not as a function the C function calls, so what it does for the script (the objects it makes
/// through mlua) is part of the one C call a script's call to update or increment is (cost.rs LUAU_CCALL), as to a client's.
struct Library {
    lua: mlua::WeakLua,
    engine: Rc<Engine>,
    store: Rc<Store>,
    bridge: LuaTable,
}

/// What a step of update's or increment's Rust leaves its C function: `n` values in the bridge (1..=n), or an error to raise
/// at the script's line (level 1) or as it is (level 0).
enum Step {
    Values(usize),
    Raise(Vec<u8>, c_int),
}

impl Library {
    /// Bridge slot `slot`, taken out (the bridge holds nothing between calls).
    fn take(&self, slot: usize) -> LuaResult<LuaValue> {
        let value = self.bridge.raw_get(slot)?;
        self.bridge.raw_set(slot, LuaValue::Nil)?;
        Ok(value)
    }

    /// The call's `count` arguments, which the C function put in the bridge, taken out.
    fn args(&self, count: c_int) -> LuaResult<Args> {
        let values = (1..=count as usize).map(|slot| self.take(slot)).collect::<LuaResult<Vec<_>>>()?;
        Ok(Args::new(LuaMultiValue::from_vec(values)))
    }

    /// Runs a step of the Rust under a C function: the values it gives are put in the bridge, its error (or a panic's message, as
    /// raw.rs's `guard` gives one) is left to raise. No Rust value of the step outlives it.
    fn step(&self, body: impl FnOnce(&Lua) -> Result<Vec<LuaValue>, Flow>) -> Step {
        let run = || -> Step {
            let Some(lua) = self.lua.try_upgrade() else { return Step::Raise(b"the engine has ended".to_vec(), 0) };
            let values = match body(&lua) {
                Ok(values) => values,
                Err(Flow::Error(error)) => return Step::Raise(error.message.into_bytes(), error.level.into()),
                Err(Flow::Yield) => return Step::Raise(b"SharedTable functions cannot yield".to_vec(), 0),
                Err(Flow::Fail(error)) => return Step::Raise(error.to_string().into_bytes(), 0),
                Err(Flow::Raised(value)) => return Step::Raise(Flow::raised_text(&value).into_bytes(), 0),
            };
            let count = values.len();
            for (slot, value) in values.into_iter().enumerate() {
                if let Err(error) = self.bridge.raw_set(slot + 1, value) {
                    return Step::Raise(error.to_string().into_bytes(), 0);
                }
            }
            Step::Values(count)
        };
        match catch_unwind(AssertUnwindSafe(run)) {
            Ok(step) => step,
            Err(panic) if super::refs::is_exhaustion(&*panic) => Step::Raise(super::refs::message().into_bytes(), 0),
            Err(panic) => {
                let message = panic.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| panic.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
                Step::Raise(format!("engine error: {message}").into_bytes(), 0)
            }
        }
    }
}

/// Puts the values at `from..=to` on the stack in the bridge of the running C function, at 1, 2, ...
unsafe fn bridge_in(state: *mut ffi::lua_State, from: c_int, to: c_int) {
    unsafe {
        for (slot, index) in (from..=to).enumerate() {
            ffi::lua_pushvalue(state, index);
            ffi::lua_rawseti_(state, ffi::lua_upvalueindex(2), slot as c_int + 1);
        }
    }
}

/// Pushes bridge slot `slot` of the running C function, and empties it.
unsafe fn bridge_take(state: *mut ffi::lua_State, slot: c_int) {
    unsafe {
        ffi::lua_rawgeti_(state, ffi::lua_upvalueindex(2), slot);
        ffi::lua_pushnil(state);
        ffi::lua_rawseti_(state, ffi::lua_upvalueindex(2), slot);
    }
}

/// Raises `message` at the script's line (level 1) or as it is (level 0), the message dropped first.
unsafe fn raise_bytes(state: *mut ffi::lua_State, message: Vec<u8>, level: c_int) -> ! {
    unsafe {
        ffi::lua_pushlstring_(state, message.as_ptr().cast(), message.len());
        drop(message);
        raw::raise_top(state, level)
    }
}

/// The library of the running C function (its first upvalue).
unsafe fn library<'a>(state: *mut ffi::lua_State) -> &'a Library {
    unsafe { &*(ffi::lua_touserdata(state, ffi::lua_upvalueindex(1)) as *const Library) }
}

/// An update in progress (SharedTable.update: [`update_c`]): its table, its key (looked at and hashed once, in
/// [`update_begin`], its string borrowed as a call's key is), the values on the stack below its callback's result (update's
/// arguments and the result: [`Conversion::new`]), and the table's version when update took its copy of the entries and the
/// copy itself, which update holds while its callback runs. A local of update's C function, which holds no other Rust value
/// while Luau runs: it is dropped as update returns, and as an error the callback raises passes through update (see the
/// comment at the callback's call in [`update_c`]), so update lets go of its copy as it leaves, as a client's does.
///
/// Roblox calls the callback again, with the value then current, whenever a write, increment, clear or update of the table
/// completed while it ran, and stores the result only of a call during which none did (measured in a playing client, Studio
/// 0.748, a callback that changes the table on its first call only: a write of the same key, of another key, of the same value
/// again, of nil or a thread to a key the table does not hold, an increment, a nested update (one storing nothing too) and a
/// clear (of an empty table too) each give two calls, the second seeing the value then current; a read, a clone, a write to
/// another SharedTable and a call that raised one: [`Shared::version`]). Its result is converted right after the call, so one
/// that cannot be stored raises before that check, and the change the callback made stays (measured: a callback that wrote the
/// table and returned a function raised "Functions can not be stored in a SharedTable" after one call). A callback that
/// changes the table on every call is called again and again (a client hung on one); here the run's script timeout ends it,
/// when the run has one, and each call after the first is charged for looking the key up again, its bytes hashed (not
/// measured: every lookup a client makes hashes its key, measured for reads). While it runs, update holds a copy of the
/// entries, as Roblox's does, so its write copies the key's path and moves a key that shares its hash first (measured: 0.58,
/// 0.78 and 1.2 us at 10, 1,000 and 100,000 entries, about a first write after a clone, and a key of a list moved first on its
/// own list).
struct Updating {
    shared: Rc<Shared>,
    key: Probe,
    height: usize,
    version: u64,
    snapshot: Option<Entries>,
}

impl Updating {
    /// The copy for a call of the callback, and the value that call gets.
    fn copy(&mut self, lua: &Lua, engine: &Engine, store: &Store) -> LuaResult<LuaValue> {
        self.version = self.shared.version.get();
        let snapshot = self.shared.entries.borrow().clone();
        let old = read(lua, engine, store, &snapshot, &self.key)?;
        self.snapshot = Some(snapshot);
        Ok(old)
    }
}

/// SharedTable.update's checks, in a client's order, and the copy for its callback's first call (`updating`): the value the
/// callback gets. `count` is the call's count of arguments, of which only the first three are in the bridge ([`update_c`]):
/// they are all update looks at, and give the same answers to its checks (an argument missing or nil).
fn update_begin(lua: &Lua, library: &Library, count: c_int, updating: &mut Option<Updating>) -> Result<Vec<LuaValue>, Flow> {
    let (engine, store) = (&*library.engine, &*library.store);
    let args = library.args(count.min(3))?;
    let shared = shared_arg(engine, &args, 0, "update")?;
    let key = key_of(args.get(1))?;
    charge(engine, key.price());
    let LuaValue::Function(_) = args.get(2) else { return raise("Third argument to SharedTable::Update should be a function.") };
    if shared.is_frozen() {
        return raise(FROZEN);
    }
    // (an update whose checks passed marks its table before the first call: [`Shared::mark`])
    shared.mark();
    let updating = updating.insert(Updating { shared, key, height: count as usize + 1, version: 0, snapshot: None });
    Ok(vec![updating.copy(lua, engine, store)?])
}

/// The callback's result for the update in progress (bridge slot 1): stored when no write, increment, clear or update of the
/// table completed during the call (no value), else the value then current for another call.
fn update_commit(lua: &Lua, library: &Library, updating: &mut Option<Updating>) -> Result<Vec<LuaValue>, Flow> {
    let (engine, store) = (&*library.engine, &*library.store);
    let result = library.take(1)?;
    let Some(updating) = updating else { return raise("SharedTable.update has no update in progress") };
    // (the result converted first, above update's arguments, the copy still held: it raises before the change check)
    let value = convert_value(lua, engine, store, &result, updating.height, 0.0);
    let snapshot = updating.snapshot.take();
    let (value, Some(mut entries)) = (value?, snapshot) else { return raise("SharedTable.update holds no copy") };
    if updating.shared.version.get() != updating.version {
        drop((value, entries));
        charge(engine, updating.key.price());
        return Ok(vec![updating.copy(lua, engine, store)?]);
    }
    // (stored in update's copy, which then becomes the table's entries: the table has not changed since the copy was taken,
    // as every change counts but a failed increment's key with no value, which the table, marked, no longer gets)
    let replaced = put(engine, &mut entries, &updating.key, value);
    let before = updating.shared.entries.replace(entries);
    updating.shared.changed();
    drop((replaced, before));
    Ok(Vec::new())
}

/// SharedTable.update: one C function, as a client's is (measured, Studio 0.748, a playing client, two runs: `debug.info` gives
/// `[C]` and `update`; in the callback, `debug.info(2, "f")` is update itself and level 3 its caller; an error the callback
/// raises passes through update as raised, a table too, so an `xpcall` handler runs at the raise, the callback's line first in
/// its traceback, and so does the Output's stack of an uncaught one). Its Rust ([`update_begin`], [`update_commit`]) runs in
/// this frame ([`Library`]); the callback is called unprotected, one C call deep as a client's is (measured: updates nested in
/// their callbacks reach depth 199 in a client's script, then raise "C stack overflow", the VM's message at its limit of 200 C
/// calls; here they stop at the same limit, at a depth that depends on the C calls the thread already holds), where it cannot
/// yield (its count of C calls is above the thread's base).
pub(crate) unsafe extern "C-unwind" fn update_c(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let library = library(state);
        let count = ffi::lua_gettop(state);
        ffi::luaL_checkstack(state, 4, c"update".as_ptr());
        // (update looks at its first three arguments only: the bridge takes no more, however many a script passes)
        bridge_in(state, 1, count.min(3));
        let mut updating = None;
        if let Step::Raise(message, level) = library.step(|lua| update_begin(lua, library, count, &mut updating)) {
            drop(updating);
            raise_bytes(state, message, level)
        }
        // (the value the callback gets, above the arguments)
        let value = count + 1;
        bridge_take(state, 1);
        loop {
            // (the callback, update's third argument, which update_begin checked)
            ffi::lua_pushvalue(state, 3);
            ffi::lua_pushvalue(state, value);
            // (`updating` is alive across this call, unlike raw.rs's rule for raw C functions that no Rust value be alive at a
            // raise: an error the callback raises unwinds through this frame and drops it, letting go of update's copy and of
            // the key's string. That is sound only because this build's Luau raises C++ exceptions (luau0-src's `use_longjmp`
            // is false) and Rust runs its drops for them on their way through `C-unwind` frames; a longjmp would skip them,
            // which is undefined behaviour. `an_update_whose_callback_raised_lets_go_of_its_copy` in src/tests.rs guards it.
            // update's own raises below drop `updating` first, as raw.rs's rule says.)
            ffi::lua_call(state, 1, 1);
            // (its result, for update_commit)
            bridge_in(state, value + 1, value + 1);
            ffi::lua_settop(state, value);
            match library.step(|lua| update_commit(lua, library, &mut updating)) {
                Step::Values(0) => {
                    drop(updating);
                    return 0;
                }
                Step::Values(_) => {
                    bridge_take(state, 1);
                    ffi::lua_replace(state, value);
                }
                Step::Raise(message, level) => {
                    drop(updating);
                    raise_bytes(state, message, level)
                }
            }
        }
    }
}

/// SharedTable.increment: one C function, as a client's is (measured, Studio 0.748, a playing client: `debug.info` gives `[C]`
/// and `increment`, and an `xpcall` handler's traceback of its error has no frame of its own), over [`increment`], which runs in
/// this frame ([`Library`]): the old value, or its error raised from here at the script's line (level 1) or as it is (level 0:
/// a message made of the key's own bytes).
pub(crate) unsafe extern "C-unwind" fn increment_c(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let library = library(state);
        let count = ffi::lua_gettop(state);
        ffi::luaL_checkstack(state, 4, c"increment".as_ptr());
        // (increment looks at its first three arguments only, and they give the same answers to its checks: a delta left out)
        bridge_in(state, 1, count.min(3));
        let step = library.step(|lua| {
            let args = library.args(count.min(3))?;
            Ok(increment(lua, &library.engine, &args)?.into_vec())
        });
        if let Step::Raise(message, level) = step {
            raise_bytes(state, message, level)
        }
        // (false and the old value, or true, the problem and its level)
        for slot in 1..=3 {
            bridge_take(state, slot);
        }
        if ffi::lua_toboolean(state, -3) == 0 {
            ffi::lua_pop(state, 1);
            return 1;
        }
        let level = ffi::lua_tointeger(state, -1) as c_int;
        ffi::lua_pop(state, 1);
        raw::raise_top(state, level)
    }
}

/// A C closure of `function`, named `name` for `debug.info` and errors, over a [`Library`] (its first upvalue) and the library's
/// bridge (its second).
fn library_closure(lua: &Lua, engine: &Rc<Engine>, store: &Rc<Store>, function: ffi::lua_CFunction, name: &'static CStr) -> LuaResult<LuaFunction> {
    let bridge = lua.create_table()?;
    let library = Library { lua: lua.weak(), engine: engine.clone(), store: store.clone(), bridge: bridge.clone() };
    // SAFETY: the bridge is pushed as the argument and the library's userdata made beside it, both taken by the closure, which is
    // left as the one result
    unsafe {
        lua.exec_raw(bridge, move |state| {
            ffi::lua_newuserdata_t(state, library);
            ffi::lua_insert(state, -2);
            ffi::lua_pushcclosurek(state, function, name.as_ptr(), 2, None);
        })
    }
}

/// SharedTable.increment: adds `delta` to the number at `key` and returns the old one, as (false, old); a missing or
/// non-number entry gives (true, message, 0), the message made of the key's own bytes for [`increment_c`] to raise as it
/// is. Its checks, in a client's order (measured, Studio 0.748, a playing client): the key, a delta left out (not passed at
/// all, as against nil), the frozen flag, the delta's type, then the entry.
fn increment(lua: &Lua, engine: &Engine, args: &Args) -> Result<LuaMultiValue, Flow> {
    let shared = shared_arg(engine, args, 0, "increment")?;
    let key = key_of(args.get(1))?;
    charge(engine, key.price());
    if args.len() < 3 {
        return raise("Failed to get value for insertion into SharedTable");
    }
    if shared.is_frozen() {
        return raise(FROZEN);
    }
    let delta = match args.get(2) {
        LuaValue::Integer(n) => *n as f64,
        LuaValue::Number(n) => *n,
        _ => return raise("Variant cast failed"),
    };
    const MISSING: &str = "it does not exist in the SharedTable";
    let mut work = Work::default();
    let found = match shared.entries.borrow().locate(key.key(), &mut work) {
        None => Err((MISSING, true)),
        Some((Stored::Nil, _)) => Err((MISSING, false)),
        Some((Stored::Number(n), at)) => Ok((*n, at)),
        Some(_) => Err(("it is not a number value", false)),
    };
    let (old, at) = match found {
        Ok(found) => found,
        Err((reason, absent)) => {
            // (a key it did not find is left with no value in a table no engine value was made of, as a client leaves it
            // (measured: [`Shared::mark`]), its add priced as a write's work, the list it joins searched once: the add's search
            // is counted, not the one that missed; not a change for update)
            if absent && !shared.marked.get() {
                work = Work::default();
                shared.entries.borrow_mut().set(key.key(), Some(Stored::Nil), &mut work);
            }
            // (the message quotes the key, a string made for the script)
            let message = increment_message(&key, reason);
            charge(engine, work.price(key.is_string()) + made_string(message.len()));
            return Ok(values([LuaValue::Boolean(true), LuaValue::String(lua.create_string(&message)?), LuaValue::Integer(0)]));
        }
    };
    // (a key in a list is written at the place found, the list searched once)
    shared.entries.borrow_mut().replace(key.key(), Stored::Number(old + delta), at, &mut work);
    shared.changed();
    // (what Studio spent on it, its write included: cost.rs; and the nodes copied from a shallow copy, as a write's)
    charge(engine, cost::SHARED_INCREMENT + work.price(key.is_string()));
    Ok(values([LuaValue::Boolean(false), LuaValue::Number(old)]))
}

/// Registers the SharedTable type and its library (types.rs).
pub fn register(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let store = Rc::new(Store {
        pool: lua.create_table()?,
        released: Rc::default(),
        free: RefCell::default(),
        next: Cell::new(0),
        tables: RefCell::default(),
        tables_kept: Cell::new(1024),
        copy_proxy: RefCell::default(),
    });

    let (e, s) = (engine.clone(), store.clone());
    let index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        let shared = this(&args)?;
        let key = key_of(args.get(1))?;
        charge(&e, key.price());
        Ok(values([read(lua, &e, &s, &shared.entries.borrow(), &key)?]))
    })?;
    let (e, s) = (engine.clone(), store.clone());
    let newindex = wrapped(lua, engine, "__newindex", move |lua, args| {
        let args = Args::new(args);
        let shared = this(&args)?;
        let key = key_of(args.get(1))?;
        write(lua, &e, &s, &shared, &key, args.get(2), 3, key.price())?;
        Ok(none())
    })?;
    let (e, s) = (engine.clone(), store.clone());
    let iter = super::lua_fn(lua, move |lua, handle: LuaAnyUserData| {
        // (the entries as they are now, shared, walked an entry a step: a loop stopped at its first entry costs Roblox the same at
        // any size, measured 0.24 to 0.73 us from 3 to 200,000 entries, and a step its entry, its key's string and its value)
        let entries = handle.borrow::<LSharedTable>()?.0.entries.borrow().clone();
        charge(&e, cost::SHARED_LOOP);
        let mut walk = Walk { numbers: Cursor::new(&entries.numbers), strings: Cursor::new(&entries.strings), _started: entries };
        let (e, s) = (e.clone(), s.clone());
        lua.create_function_mut(move |lua, _: LuaMultiValue| {
            // (the whole walk, the entries it started on too: a closure that named only its cursors would take only those)
            let walk = &mut walk;
            if let Some((n, value)) = walk.numbers.next() {
                charge(&e, cost::SHARED_LOOP_INTEGER_ENTRY);
                return Ok((LuaValue::Number(f64::from(n)), load(lua, &e, &s, &value, false)?));
            }
            if let Some((key, value)) = walk.strings.next() {
                charge(&e, cost::SHARED_LOOP_STRING_ENTRY + made_string(key.len()));
                return Ok((LuaValue::String(lua.create_string(&*key)?), load(lua, &e, &s, &value, false)?));
            }
            Ok((LuaValue::Nil, LuaValue::Nil))
        })
    })?;
    let tostring = super::lua_fn(lua, |_, handle: LuaAnyUserData| Ok(format!("SharedTable: 0x{:016x}", handle.to_pointer() as usize)))?;
    // (two handles are equal when they are of one SharedTable: [`handle`])
    let equal = super::create_boxed_val2(lua, Box::new(|_, a, b| {
        let table = |value: &LuaValue| match value {
            LuaValue::UserData(ud) => ud.borrow::<LSharedTable>().ok().map(|held| Rc::as_ptr(&held.0)),
            _ => None,
        };
        Ok(LuaValue::Boolean(matches!((table(&a), table(&b)), (Some(a), Some(b)) if a == b)))
    }))?;
    lua.register_userdata_type::<LSharedTable>(|registry| {
        registry.add_meta_field("__type", "SharedTable");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__iter", iter);
        registry.add_meta_field("__tostring", tostring);
        registry.add_meta_field("__eq", equal);
    })?;
    lock_metatable(lua, &lua.create_any_userdata(LSharedTable(Shared::new(Entries::default(), false)))?)?;

    let library = lua.create_table()?;
    let (e, s) = (engine.clone(), store.clone());
    library.set(
        "new",
        super::library_function(lua, "new", move |lua, args| {
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
                other => return raise_at_caller(format!("invalid argument #1 to 'new' (table expected, got {})", typeof_value(&e, other))),
            };
            Ok(values([LuaValue::UserData(handle(lua, &shared)?)]))
        })?,
    )?;
    for (name, frozen) in [("clone", false), ("cloneAndFreeze", true)] {
        let (e, s) = (engine.clone(), store.clone());
        library.set(
            name,
            super::library_function(lua, name, move |lua, args| {
                let args = Args::new(args);
                let deep = deep_arg(&e, &args, name)?;
                let shared = shared_arg(&e, &args, 0, name)?;
                let mut tally = Tally::default();
                let copied = copy(&s, &shared, deep, frozen, &mut tally, &mut bindable::Tally::default());
                // (a shallow copy shares the entries, and costs Studio the same at any size: only the call; a deep one is charged for
                // the tables it copied also when it stops at ree's guard)
                charge(
                    &e,
                    tally.strings as f64 * cost::SHARED_DEEP_STRING_ENTRY + tally.integers as f64 * cost::SHARED_DEEP_INTEGER_ENTRY + tally.tables as f64 * cost::SHARED_DEEP_TABLE,
                );
                let copied = match copied {
                    Ok(copied) => copied,
                    Err(message) => return raise(message),
                };
                Ok(values([LuaValue::UserData(handle(lua, &copied)?)]))
            })?,
        )?;
    }
    let e = engine.clone();
    library.set("isFrozen", super::library_function(lua, "isFrozen", move |_, args| Ok(values([LuaValue::Boolean(shared_arg(&e, &Args::new(args), 0, "isFrozen")?.is_frozen())])))?)?;
    // (clear set before size: a plain table's node order, so the library loops as a client's does (measured: cloneAndFreeze,
    // clear, clone, isFrozen, size, increment, update, new))
    let e = engine.clone();
    library.set(
        "clear",
        super::library_function(lua, "clear", move |_, args| {
            let shared = shared_arg(&e, &Args::new(args), 0, "clear")?;
            if shared.is_frozen() {
                return raise(FROZEN);
            }
            // (taken out first: what it held may let go of a pool slot; a change for update, an empty table's too)
            let cleared = shared.entries.take();
            shared.changed();
            // (charged for the entries it frees, those no copy shares: cost.rs SHARED_CLEARED_ENTRY)
            let freed = release(cleared);
            charge(&e, freed as f64 * cost::SHARED_CLEARED_ENTRY);
            Ok(none())
        })?,
    )?;
    let e = engine.clone();
    library.set(
        "size",
        super::library_function(lua, "size", move |_, args| Ok(values([LuaValue::Number(shared_arg(&e, &Args::new(args), 0, "size")?.entries.borrow().len() as f64)])))?,
    )?;
    // (increment and update are C functions, as a client's are: lua/raw.rs marks them as the engine's own, so a script's call to
    // one is one C call)
    library.set("increment", library_closure(lua, engine, &store, increment_c, c"increment")?)?;
    library.set("update", library_closure(lua, engine, &store, update_c, c"update")?)?;
    library.set_readonly(true);
    engine.lua.borrow_mut().libraries.insert("SharedTable", library);
    Ok(())
}

#[cfg(test)]
mod tests {
    use rustc_hash::{FxHashMap, FxHashSet};

    use super::*;

    /// A key as the tests name it, owned (a string's bytes hashed when it is looked for).
    #[derive(Clone, Debug)]
    enum Key {
        Number(u32),
        String(Rc<[u8]>),
    }

    impl Key {
        fn view(&self) -> KeyRef<'_> {
            match self {
                Key::Number(n) => KeyRef::Number(*n),
                Key::String(s) => KeyRef::String(s, fnv1a(s)),
            }
        }
    }

    impl Entries {
        /// [`Entries::set`] by a test's key: the value replaced, and the nodes copied because another copy shared them.
        fn put(&mut self, key: Key, value: Option<Stored>) -> (Option<Stored>, usize) {
            let mut work = Work::default();
            let replaced = self.set(key.view(), value, &mut work);
            (replaced, work.copied)
        }

        fn at(&self, key: &Key) -> Option<&Stored> {
            self.get(key.view(), &mut Work::default())
        }

        /// The entries in the order a loop walks them.
        fn ordered(&self) -> Vec<(Key, Stored)> {
            let numbers = Cursor::new(&self.numbers).map(|(n, value)| (Key::Number(n), value));
            numbers.chain(Cursor::new(&self.strings).map(|(s, value)| (Key::String(s), value))).collect()
        }
    }

    /// A key with few bits, to make keys whose bits are all equal: its first byte is its bits (a slot of 5 bits, then one of 3,
    /// then a list), its second tells keys of equal bits apart.
    #[derive(Clone, PartialEq, Debug)]
    struct Tiny(u8, u8);

    impl TrieKey for Tiny {
        type Ref = Tiny;
        const IN_ORDER: bool = false;
        fn view(&self) -> &Tiny {
            self
        }
        fn make(key: &Tiny) -> Tiny {
            key.clone()
        }
        fn bytes(_: &Tiny) -> usize {
            0
        }
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
        fn compare(a: &Tiny, b: &Tiny) -> (bool, usize) {
            COMPARED.set(COMPARED.get() + 1);
            (a == b, 0)
        }
    }

    thread_local! {
        /// How many times [`Tiny`] keys were compared
        static COMPARED: Cell<usize> = const { Cell::new(0) };
    }

    impl Trie<Tiny> {
        fn put(&mut self, key: Tiny, value: Stored) -> Option<Stored> {
            let bits = key.bits();
            self.insert(&key, bits, value, &mut Work::default())
        }

        fn take(&mut self, key: &Tiny) -> Option<Stored> {
            self.remove(key, key.bits(), &mut Work::default())
        }

        fn at(&self, key: &Tiny) -> Option<&Stored> {
            self.get(key, key.bits(), &mut Work::default())
        }

        fn keys(&self) -> Vec<Tiny> {
            Cursor::new(self).map(|(key, _)| key).collect()
        }
    }

    /// The order Roblox walks string keys, worked out from the keys alone (measured: [`TrieKey`] for strings): sorted by their
    /// slot level by level, a level's lone keys before its groups. The trie must walk in this order after any writes.
    fn string_order(keys: Vec<&[u8]>) -> Vec<&[u8]> {
        fn group_order<'a>(group: &mut [(u64, &'a [u8])], level: u32, order: &mut Vec<&'a [u8]>) {
            if group.len() <= 1 || level * 5 >= 64 {
                crate::sort::by(group, |a, b| a.1.cmp(b.1));
                order.extend(group.iter().map(|(_, key)| *key));
                return;
            }
            let slot = |hash: u64| (hash >> (5 * level)) & 31;
            crate::sort::by_key(group, |(hash, _)| slot(*hash));
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

    fn checked(entries: &Entries, numbers: &FxHashMap<u32, f64>, strings: &FxHashMap<Vec<u8>, f64>) {
        assert_eq!((entries.numbers.len, entries.strings.len), (numbers.len(), strings.len()));
        for (n, value) in numbers {
            assert_eq!(number(entries.at(&Key::Number(*n))), Some(*value));
        }
        for (s, value) in strings {
            assert_eq!(number(entries.at(&Key::String(s.as_slice().into()))), Some(*value));
        }
        let ordered = entries.ordered();
        let walked_numbers: Vec<u32> = ordered.iter().filter_map(|(key, _)| if let Key::Number(n) = key { Some(*n) } else { None }).collect();
        let walked_strings: Vec<&[u8]> = ordered.iter().filter_map(|(key, _)| if let Key::String(s) = key { Some(&**s) } else { None }).collect();
        let mut expected_numbers: Vec<u32> = numbers.keys().copied().collect();
        crate::sort::ord(&mut expected_numbers);
        assert_eq!(walked_numbers, expected_numbers);
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
        let (mut numbers, mut strings) = (FxHashMap::default(), FxHashMap::default());
        for step in 0..20_000u32 {
            let roll = next();
            let value = f64::from(step);
            let remove = roll % 3 == 0;
            if roll & 16 == 0 {
                let key = format!("k{}", (roll >> 8) % 3000).into_bytes();
                if remove {
                    entries.put(Key::String(key.as_slice().into()), None);
                    strings.remove(&key);
                } else {
                    entries.put(Key::String(key.as_slice().into()), Some(Stored::Number(value)));
                    strings.insert(key, value);
                }
            } else {
                // (small integers, and ones spread over the whole range)
                let key = if roll & 32 == 0 { ((roll >> 8) % 3000) as u32 } else { (roll >> 32) as u32 };
                if remove {
                    entries.put(Key::Number(key), None);
                    numbers.remove(&key);
                } else {
                    entries.put(Key::Number(key), Some(Stored::Number(value)));
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
            entries.put(Key::Number(key), None);
        }
        for key in strings.keys().cloned().collect::<Vec<_>>() {
            entries.put(Key::String(key.as_slice().into()), None);
        }
        assert!(entries.numbers.root.is_none() && entries.strings.root.is_none() && entries.len() == 0);
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
            entries.put(Key::Number(n), Some(Stored::Number(f64::from(n))));
            entries.put(Key::String(format!("k{n}").into_bytes().into()), Some(Stored::Number(f64::from(n))));
        }
        let original = Shared::new(entries, false);
        let copied = Shared::new(original.entries.borrow().clone(), true);
        assert_eq!(all_nodes(&original.entries.borrow()), all_nodes(&copied.entries.borrow()));
        let (_, first) = copied.entries.borrow_mut().put(Key::Number(0), Some(Stored::Boolean(true)));
        let (_, again) = copied.entries.borrow_mut().put(Key::Number(1), Some(Stored::Boolean(true)));
        let (_, removed) = copied.entries.borrow_mut().put(Key::String(b"k0"[..].into()), None);
        // (each write counts the shared nodes it copied: none the second time down the same path)
        assert!(first >= 2 && again == 0 && removed >= 1, "{first} {again} {removed}");
        let shared: FxHashSet<*const ()> = all_nodes(&original.entries.borrow()).into_iter().collect();
        let fresh = all_nodes(&copied.entries.borrow()).into_iter().filter(|node| !shared.contains(node)).count();
        // (at most one node a level on each of the two paths: seven levels for integers, thirteen for strings)
        // (a removal may fold a subnode back into its parent: no more fresh nodes than the copies counted)
        assert!((2..=20).contains(&fresh) && first <= fresh && fresh <= first + removed, "{fresh} nodes copied");
        assert_eq!(number(original.entries.borrow().at(&Key::Number(0))), Some(0.0));
        assert_eq!(number(original.entries.borrow().at(&Key::String(b"k0"[..].into()))), Some(0.0));
        assert!(matches!(copied.entries.borrow().at(&Key::Number(0)), Some(Stored::Boolean(true))));
        assert_eq!(number(original.entries.borrow().at(&Key::Number(1))), Some(1.0));
        assert!(copied.entries.borrow().at(&Key::String(b"k0"[..].into())).is_none());
        assert_eq!((original.entries.borrow().len(), copied.entries.borrow().len()), (20_000, 19_999));
        assert_eq!(number(copied.entries.borrow().at(&Key::Number(2))), Some(2.0));
        assert!(copied.is_frozen() && !original.is_frozen());
    }

    fn store(lua: &Lua) -> Store {
        Store {
            pool: lua.create_table().unwrap(),
            released: Rc::default(),
            free: RefCell::default(),
            next: Cell::new(0),
            tables: RefCell::default(),
            tables_kept: Cell::new(1024),
            copy_proxy: RefCell::default(),
        }
    }

    /// A pooled value no SharedTable holds any more leaves the pool at the next read ([`load`] drains it), not only at the next
    /// value pooled: a script that stops writing pooled values still gets their memory back.
    #[test]
    fn a_pooled_value_let_go_of_leaves_the_pool_at_the_next_read() {
        let lua = Lua::new();
        let store = store(&lua);
        let held = store.put(LuaValue::String(lua.create_string("held").unwrap()), 0.0).unwrap();
        let dropped = store.put(LuaValue::String(lua.create_string("dropped").unwrap()), 0.0).unwrap();
        let (held_id, dropped_id) = (held.id, dropped.id);
        drop(dropped);
        assert!(!store.pool.raw_get::<LuaValue>(dropped_id).unwrap().is_nil(), "kept until the pool is drained");
        store.drain().unwrap();
        assert!(store.pool.raw_get::<LuaValue>(dropped_id).unwrap().is_nil(), "let go of");
        assert!(!store.pool.raw_get::<LuaValue>(held_id).unwrap().is_nil(), "a value still held stays");
        assert_eq!(*store.free.borrow(), vec![dropped_id]);
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
            entries.put(Key::String(link.clone()), Some(Stored::Shared(head)));
            head = store.table(entries, false);
        }
        let mut tally = Tally::default();
        let copied = copy(&store, &head, true, true, &mut tally, &mut bindable::Tally::default()).expect("under the guard");
        assert_eq!((tally.tables, tally.strings), (300_000, 300_000));
        let mut length = 0;
        let mut at = copied.clone();
        loop {
            assert!(at.is_frozen());
            let next = match at.entries.borrow().at(&Key::String(link.clone())) {
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
        table.entries.borrow_mut().put(Key::String(b"self"[..].into()), Some(Stored::Shared(table.clone())));
        table.entries.borrow_mut().put(Key::String(b"bytes"[..].into()), Some(Stored::Buffer(vec![0; 1 << 20].into())));
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

    /// The SharedTables below `node`, recounted, each node's own count checked on the way.
    fn recount<K>(node: &Node<K>) -> usize {
        let own = node.entries.iter().filter(|(_, value)| matches!(value, Stored::Shared(_))).count();
        let below: usize = node.nodes.iter().map(|child| recount(child)).sum();
        assert_eq!(node.tables, own + below, "a node's count of the SharedTables at and below it");
        own + below
    }

    fn recounted(entries: &Entries) -> usize {
        let held = entries.numbers.root.as_deref().map_or(0, recount) + entries.strings.root.as_deref().map_or(0, recount);
        assert_eq!(entries.tables(), held);
        held
    }

    /// The slots (entries and subnodes) of the nodes a deep copy walks: those with a SharedTable at or below them.
    fn walked(entries: &Entries) -> u64 {
        fn slots<K>(node: &Node<K>) -> u64 {
            if node.tables == 0 {
                return 0;
            }
            (node.entries.len() + node.nodes.len()) as u64 + node.nodes.iter().map(|child| slots(child)).sum::<u64>()
        }
        entries.numbers.root.as_deref().map_or(0, slots) + entries.strings.root.as_deref().map_or(0, slots)
    }

    /// Each node counts the SharedTables at and below it, through any order of writes, overwrites and removals (splits and the
    /// folding back of a subnode's last entry included), and a shallow copy keeps its counts while the table it was made from
    /// changes the nodes they shared, and the other way round.
    #[test]
    fn each_node_counts_the_shared_tables_below_it() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let tables: Vec<Rc<Shared>> = (0..4).map(|_| Shared::new(Entries::default(), false)).collect();
        let mut entries = Entries::default();
        let mut copies: Vec<Entries> = Vec::new();
        for step in 0..40_000u32 {
            let roll = next();
            let value = match roll % 4 {
                0 => None,
                1 => Some(Stored::Shared(tables[(roll >> 40) as usize % 4].clone())),
                _ => Some(Stored::Number(f64::from(step))),
            };
            let key = if roll & 16 == 0 {
                Key::String(format!("k{}", (roll >> 8) % 2000).into_bytes().into())
            } else if roll & 32 == 0 {
                Key::Number(((roll >> 8) % 2000) as u32)
            } else {
                Key::Number((roll >> 32) as u32)
            };
            // (now and then the same key the other way in the last shallow copy, whose nodes the table shares)
            if roll & 64 == 0
                && let Some(copy) = copies.last_mut()
            {
                let other = if matches!(value, Some(Stored::Shared(_))) { None } else { Some(Stored::Shared(tables[0].clone())) };
                copy.put(key.clone(), other);
            }
            entries.put(key, value);
            if step % 5000 == 0 {
                copies.push(entries.clone());
            }
            if step % 2500 == 0 {
                recounted(&entries);
                for copy in &copies {
                    recounted(copy);
                }
            }
        }
        let held = recounted(&entries);
        assert!(held > 100, "{held}");
        for copy in &copies {
            recounted(copy);
        }
        // emptied one key at a time, every count comes down to none, and the copies keep theirs
        let keys: Vec<Key> = entries.ordered().into_iter().map(|(key, _)| key).collect();
        for (at, key) in keys.into_iter().enumerate() {
            entries.put(key, None);
            if at % 500 == 0 {
                recounted(&entries);
            }
        }
        assert_eq!(recounted(&entries), 0);
        for copy in &copies {
            recounted(copy);
        }
    }

    /// Keys whose bits are all equal share a list, which counts its SharedTables as a node does; a deep copy reaches the only
    /// SharedTable through it, copying the nodes on the way and the list once, and none when the table is taken out again.
    #[test]
    fn a_list_of_keys_with_equal_bits_counts_its_tables_and_a_deep_copy_reaches_them() {
        let lua = Lua::new();
        let store = store(&lua);
        let inner = store.table(Entries::default(), false);
        let mut trie: Trie<Tiny> = Trie::default();
        // (1 and 33 share slot 1 at the first level; the ones then share a list of 50, the other sits apart)
        for n in 0..50u8 {
            trie.put(Tiny(1, n), Stored::Number(f64::from(n)));
        }
        trie.put(Tiny(33, 0), Stored::Boolean(true));
        trie.put(Tiny(1, 25), Stored::Shared(inner.clone()));
        assert_eq!((recount(trie.root.as_ref().unwrap()), trie.tables()), (1, 1));
        let (mut tally, mut made) = (Tally::default(), bindable::Tally::default());
        let mut deep = DeepCopy { store: &store, frozen: false, copies: FxHashMap::default(), pending: Vec::new(), made: &mut made, tally: &mut tally };
        let root = trie.root.clone().unwrap();
        let copied = retarget(&root, &mut deep, &mut FxHashMap::default()).expect("under the guard");
        let inner_copy = deep.copies[&Rc::as_ptr(&inner)].clone();
        drop(deep);
        let copied = Trie { root: Some(copied), len: trie.len };
        assert!(matches!(copied.at(&Tiny(1, 25)), Some(Stored::Shared(table)) if Rc::ptr_eq(table, &inner_copy) && !Rc::ptr_eq(table, &inner)));
        assert!(matches!(trie.at(&Tiny(1, 25)), Some(Stored::Shared(table)) if Rc::ptr_eq(table, &inner)));
        assert!(matches!(copied.at(&Tiny(1, 24)), Some(Stored::Number(n)) if *n == 24.0));
        assert_eq!(recount(copied.root.as_ref().unwrap()), 1);
        // (made: inner's copy, the root's slot, the next node's two and the list's 50)
        assert_eq!(made.values, 1 + 1 + 2 + 50);
        // overwritten with a number, the table leaves every count on its path, and a deep copy walks nothing
        trie.put(Tiny(1, 25), Stored::Number(25.0));
        assert_eq!(recount(trie.root.as_ref().unwrap()), 0);
        let (mut tally, mut made) = (Tally::default(), bindable::Tally::default());
        let mut deep = DeepCopy { store: &store, frozen: false, copies: FxHashMap::default(), pending: Vec::new(), made: &mut made, tally: &mut tally };
        let root = trie.root.clone().unwrap();
        let same = retarget(&root, &mut deep, &mut FxHashMap::default()).expect("nothing made");
        drop(deep);
        assert!(Rc::ptr_eq(&same, &root) && made.values == 0);
        // the list's last table: emptied down to one key, which folds back up with its count
        trie.put(Tiny(1, 3), Stored::Shared(inner.clone()));
        for n in (0..50u8).filter(|n| *n != 3) {
            trie.take(&Tiny(1, n));
            assert_eq!(recount(trie.root.as_ref().unwrap()), 1);
        }
        trie.take(&Tiny(33, 0));
        let root = trie.root.as_ref().unwrap();
        assert!(root.nodes.is_empty() && root.entries.len() == 1 && root.tables == 1);
        trie.take(&Tiny(1, 3));
        assert!(trie.root.is_none() && trie.tables() == 0);
    }

    /// A deep copy of tables that share their trie (shallow clones) copies each node leading to a SharedTable once, and the copies
    /// share it as the tables shared theirs: 1,000 shallow clones of a table whose 10,000 entries hold one SharedTable copy its trie
    /// once, not 1,000 times, and that SharedTable once. Priced as Studio copies them, every entry of every table.
    #[test]
    fn a_deep_copy_shares_what_the_tables_it_copies_shared() {
        let lua = Lua::new();
        let store = store(&lua);
        let inner = store.table(Entries::default(), false);
        let mut entries = Entries::default();
        for n in 0..10_000u32 {
            entries.put(Key::Number(n), Some(Stored::Shared(inner.clone())));
        }
        entries.put(Key::String(b"x"[..].into()), Some(Stored::Number(1.0)));
        let table = store.table(entries, false);
        let mut holder = Entries::default();
        for n in 0..1000u32 {
            holder.put(Key::Number(n), Some(Stored::Shared(store.table(table.entries.borrow().clone(), false))));
        }
        let holder = store.table(holder, false);
        let (mut tally, mut made) = (Tally::default(), bindable::Tally::default());
        let copied = copy(&store, &holder, true, false, &mut tally, &mut made).expect("under the guard");
        let at = |table: &Rc<Shared>, n: u32| match table.entries.borrow().at(&Key::Number(n)) {
            Some(Stored::Shared(held)) => held.clone(),
            _ => panic!("a SharedTable at {n}"),
        };
        let root = |table: &Rc<Shared>| table.entries.borrow().numbers.root.clone().expect("entries");
        let (first, last) = (at(&copied, 0), at(&copied, 999));
        assert!(!Rc::ptr_eq(&first, &last) && Rc::ptr_eq(&root(&first), &root(&last)) && !Rc::ptr_eq(&root(&first), &root(&table)));
        let inner_copy = at(&first, 0);
        assert!(!Rc::ptr_eq(&inner_copy, &inner) && (0..10_000).all(|n| Rc::ptr_eq(&at(&last, n), &inner_copy)));
        assert_eq!(recounted(&last.entries.borrow()), 10_000);
        // (the entries that hold no SharedTable stay shared with the original)
        let strings = |table: &Rc<Shared>| table.entries.borrow().strings.root.clone().expect("a string key");
        assert!(Rc::ptr_eq(&strings(&last), &strings(&table)));
        // made: the holder, its 1,000 tables and inner's copy, and the nodes walked once each
        assert_eq!(made.values, 1002 + walked(&holder.entries.borrow()) + walked(&table.entries.borrow()));
        assert_eq!((tally.integers, tally.strings, tally.tables), (1000 * 10_000 + 1000, 1000, 1000 * 10_000 + 1000));
        // a write to one copy copies its own path, as a write after a shallow clone does; the others keep the shared trie
        let (_, copied_nodes) = last.entries.borrow_mut().put(Key::Number(5), Some(Stored::Number(5.0)));
        assert!(copied_nodes >= 2 && Rc::ptr_eq(&at(&first, 5), &inner_copy) && recounted(&last.entries.borrow()) == 9_999);
    }

    /// A deep copy walks only to the SharedTables: 1,000 shallow clones of a table of 100,000 numbers are copied as tables of
    /// their own sharing that table's trie untouched, the holder's nodes alone copied, and priced by all their entries.
    #[test]
    fn a_deep_copy_walks_only_to_the_shared_tables() {
        let lua = Lua::new();
        let store = store(&lua);
        let mut flat = Entries::default();
        for n in 0..100_000u32 {
            flat.put(Key::String(format!("k{n}").into_bytes().into()), Some(Stored::Number(f64::from(n))));
        }
        let flat = store.table(flat, false);
        let mut holder = Entries::default();
        for n in 0..1000u32 {
            holder.put(Key::Number(n), Some(Stored::Shared(store.table(flat.entries.borrow().clone(), false))));
        }
        let holder = store.table(holder, false);
        let (mut tally, mut made) = (Tally::default(), bindable::Tally::default());
        let copied = copy(&store, &holder, true, true, &mut tally, &mut made).expect("under the guard");
        let held = match copied.entries.borrow().at(&Key::Number(7)) {
            Some(Stored::Shared(held)) => held.clone(),
            _ => panic!("a SharedTable at 7"),
        };
        let root = |table: &Rc<Shared>| table.entries.borrow().strings.root.clone().expect("entries");
        assert!(held.is_frozen() && Rc::ptr_eq(&root(&held), &root(&flat)));
        assert_eq!(made.values, 1 + 1000 + walked(&holder.entries.borrow()));
        assert_eq!((tally.strings, tally.integers, tally.tables), (1000 * 100_000, 1000, 1000));
    }

    /// A deep copy counts what it makes against ree's guard on a copy, each table and each slot of each node it copies, and
    /// stops one past it with the guard's error; the copies it made are emptied, so the one that holds itself does not stay.
    #[test]
    fn a_deep_copy_stops_at_rees_guard_and_leaves_no_copy_behind() {
        let lua = Lua::new();
        let store = store(&lua);
        let e = store.table(Entries::default(), false);
        let t = store.table(Entries::default(), false);
        t.entries.borrow_mut().put(Key::String(b"a"[..].into()), Some(Stored::Shared(e.clone())));
        let s = store.table(Entries::default(), false);
        s.entries.borrow_mut().put(Key::String(b"self"[..].into()), Some(Stored::Shared(s.clone())));
        s.entries.borrow_mut().put(Key::String(b"t"[..].into()), Some(Stored::Shared(t.clone())));
        let alive = || store.tables.borrow().iter().filter(|table| table.strong_count() > 0).count();
        let before = alive();
        // (made: three tables and the slots of the nodes walked; T's copy and E's are made after S's copy holds itself)
        let makes = 3 + walked(&s.entries.borrow()) + walked(&t.entries.borrow());
        let attempt = |values: u64| copy(&store, &s, true, false, &mut Tally::default(), &mut bindable::Tally { values, bytes: 0 });
        let copied = attempt(bindable::COPY_VALUES - makes).expect("at the guard");
        assert!(matches!(copied.entries.borrow().at(&Key::String(b"self"[..].into())), Some(Stored::Shared(own)) if Rc::ptr_eq(own, &copied)));
        release(copied.entries.take());
        drop(copied);
        assert_eq!(alive(), before);
        assert_eq!(attempt(bindable::COPY_VALUES - makes + 1).err(), Some(bindable::too_large(bindable::COPY_VALUES, "values")));
        assert_eq!(alive(), before, "a failed copy leaves none of its copies");
    }

    /// Eight 48-byte keys whose 64-bit FNV-1a hashes are all 0x4b27f9ed5892fccb, real collisions (pairs of 16-byte blocks of
    /// equal hash found offline, chained): key `i` takes `COLLIDING_A[j]` or `COLLIDING_B[j]` by bit `j` of `i`.
    const COLLIDING_A: [&str; 3] = ["amacobmcimlpggfd", "gajldeibogfilcdn", "oaomcbcfamgpdfnp"];
    const COLLIDING_B: [&str; 3] = ["jadnfnainmlpjeik", "picgfcflgnjiaddi", "okpppjoaaachdced"];

    fn colliding(i: usize) -> Rc<[u8]> {
        (0..3).flat_map(|j| if i >> j & 1 == 1 { COLLIDING_B[j] } else { COLLIDING_A[j] }.bytes()).collect::<Vec<u8>>().into()
    }

    /// The colliding keys' indices in walk order.
    fn walked_indices(entries: &Entries) -> Vec<usize> {
        let keys: Vec<Rc<[u8]>> = (0..8).map(colliding).collect();
        entries.ordered().into_iter().filter_map(|(key, _)| if let Key::String(s) = key { keys.iter().position(|k| *k == s) } else { None }).collect()
    }

    /// Keys whose 64-bit hashes are equal are kept in one list as Roblox keeps them (measured in a playing client with these
    /// keys, Studio 0.748): the newest written first; a removal keeps the others' order; a key added goes first; an overwrite in
    /// the table's own list stays in place, while one in a list a shallow copy shares copies the list and moves the key first,
    /// the copy keeping its order. Each search compares from the front, and a list a key joins or leaves is rebuilt (counted).
    #[test]
    fn keys_with_one_hash_are_kept_newest_written_first() {
        assert!((0..8).all(|i| fnv1a(&colliding(i)) == 0x4b27_f9ed_5892_fccb));
        let mut entries = Entries::default();
        for i in 0..8 {
            entries.put(Key::String(colliding(i)), Some(Stored::Number(i as f64)));
        }
        assert_eq!(walked_indices(&entries), [7, 6, 5, 4, 3, 2, 1, 0]);
        entries.put(Key::String(colliding(3)), None);
        assert_eq!(walked_indices(&entries), [7, 6, 5, 4, 2, 1, 0]);
        let mut work = Work::default();
        entries.set(Key::String(colliding(3)).view(), Some(Stored::Number(3.0)), &mut work);
        // (a key added: the list searched to its end, each of the 7 compared (its 48 bytes, one block), and rebuilt)
        assert_eq!(walked_indices(&entries), [3, 7, 6, 5, 4, 2, 1, 0]);
        assert_eq!((work.scanned, work.compared, work.rebuilt, work.list_copied, work.added), (7, 7 * 48, 7, 0, 48));
        let mut work = Work::default();
        entries.set(Key::String(colliding(5)).view(), Some(Stored::Number(55.0)), &mut work);
        assert_eq!(walked_indices(&entries), [3, 7, 6, 5, 4, 2, 1, 0]);
        assert_eq!((work.rebuilt, work.list_copied, work.added), (0, 0, 0));
        // (a SharedTable in the list, counted by every node on its path through the moves below)
        entries.put(Key::String(colliding(2)), Some(Stored::Shared(Shared::new(Entries::default(), false))));
        assert_eq!(recounted(&entries), 1);
        let copy = entries.clone();
        let mut work = Work::default();
        entries.set(Key::String(colliding(0)).view(), Some(Stored::Number(-1.0)), &mut work);
        assert_eq!(walked_indices(&entries), [0, 3, 7, 6, 5, 4, 2, 1]);
        assert_eq!(walked_indices(&copy), [3, 7, 6, 5, 4, 2, 1, 0]);
        assert_eq!((work.list_copied, work.rebuilt, work.scanned), (8, 0, 8));
        assert_eq!(number(copy.at(&Key::String(colliding(0)))), Some(0.0));
        assert_eq!(number(entries.at(&Key::String(colliding(0)))), Some(-1.0));
        // (a removal from a list a copy shares keeps the others' order; the copy keeps its list)
        let copy = entries.clone();
        entries.put(Key::String(colliding(6)), None);
        assert_eq!(walked_indices(&entries), [0, 3, 7, 5, 4, 2, 1]);
        assert_eq!(walked_indices(&copy), [0, 3, 7, 6, 5, 4, 2, 1]);
        assert_eq!((recounted(&entries), recounted(&copy)), (1, 1));
        let copy = entries.clone();
        entries.put(Key::String(colliding(2)), Some(Stored::Number(2.0)));
        assert_eq!(walked_indices(&entries), [2, 0, 3, 7, 5, 4, 1]);
        assert_eq!((recounted(&entries), recounted(&copy)), (0, 1));
        // (one key left moves back up to the root, a slot of its own)
        for i in [0, 3, 7, 5, 4, 2] {
            entries.put(Key::String(colliding(i)), None);
            recounted(&entries);
        }
        let root = entries.strings.root.as_ref().expect("a key");
        assert!(root.nodes.is_empty() && root.entries.len() == 1 && number(entries.at(&Key::String(colliding(1)))) == Some(1.0));
    }

    /// Keys whose bits are all equal share a list past the last level, the newest first; when one is left, it moves back up.
    #[test]
    fn keys_with_equal_bits_share_a_list_and_leave_it_canonically() {
        let mut trie = Trie::default();
        for key in [Tiny(1, 5), Tiny(33, 0), Tiny(1, 9), Tiny(1, 3)] {
            trie.put(key, Stored::Boolean(true));
        }
        // (1 and 33 share slot 1 at the first level; 33 has the second level's slot 1 alone, the ones share slot 0 and then a list,
        // the newest first)
        assert_eq!(trie.keys(), [Tiny(33, 0), Tiny(1, 3), Tiny(1, 9), Tiny(1, 5)]);
        assert!(trie.at(&Tiny(1, 9)).is_some() && trie.at(&Tiny(1, 4)).is_none());
        // (a key the list does not hold takes nothing out of it)
        assert!(remove(trie.root.as_mut().unwrap(), &Tiny(1, 4), Tiny(1, 4).bits(), 0, None, &mut Work::default()).is_none() && trie.len == 4);
        assert_eq!(trie.keys(), [Tiny(33, 0), Tiny(1, 3), Tiny(1, 9), Tiny(1, 5)]);
        trie.take(&Tiny(1, 3));
        trie.take(&Tiny(1, 9));
        assert_eq!(trie.keys(), [Tiny(1, 5), Tiny(33, 0)]);
        trie.take(&Tiny(33, 0));
        let root = trie.root.as_ref().unwrap();
        assert!(root.nodes.is_empty() && root.entries.len() == 1 && root.entry_slots == 1 << 1);
    }

    /// A key is looked up from the bytes a call borrows, at a slot and in a list, its bytes hashed once; only a key a write adds
    /// is copied in (an overwrite keeps the key held), and a list's search counts each entry it compared.
    #[test]
    fn a_borrowed_key_finds_what_was_stored_and_only_an_added_key_is_copied() {
        let mut entries = Entries::default();
        for i in 0..8 {
            entries.put(Key::String(colliding(i)), Some(Stored::Number(i as f64)));
        }
        entries.put(Key::String(b"plain"[..].into()), Some(Stored::Number(9.0)));
        let held = |entries: &Entries, key: &[u8]| entries.ordered().into_iter().find_map(|(k, _)| if let Key::String(s) = k { (&*s == key).then_some(s) } else { None }).unwrap();
        for i in 0..8 {
            let bytes: Vec<u8> = colliding(i).to_vec();
            assert_eq!(number(entries.get(KeyRef::String(&bytes, fnv1a(&bytes)), &mut Work::default())), Some(i as f64));
        }
        let plain = b"plain".to_vec();
        let mut work = Work::default();
        assert_eq!(number(entries.get(KeyRef::String(&plain, fnv1a(&plain)), &mut work)), Some(9.0));
        assert_eq!((work.scanned, work.compared), (0, 0));
        // (a key the list does not hold, of its hash: every entry compared; same length, so the 48 bytes read up to the block that differs)
        let mut missing = colliding(0).to_vec();
        missing[47] ^= 1;
        let mut work = Work::default();
        assert!(entries.get(KeyRef::String(&missing, 0x4b27_f9ed_5892_fccb), &mut work).is_none());
        assert_eq!((work.scanned, work.compared), (8, 8 * 48));
        // an overwrite keeps the key held, at a slot and in a list; a key added is copied once
        let before = (held(&entries, b"plain"), held(&entries, &colliding(4)));
        let mut work = Work::default();
        entries.set(KeyRef::String(&plain, fnv1a(&plain)), Some(Stored::Number(10.0)), &mut work);
        let four = colliding(4).to_vec();
        entries.set(KeyRef::String(&four, fnv1a(&four)), Some(Stored::Number(40.0)), &mut work);
        assert_eq!(work.added, 0);
        assert!(Rc::ptr_eq(&before.0, &held(&entries, b"plain")) && Rc::ptr_eq(&before.1, &held(&entries, &colliding(4))));
        let fresh = b"fresh".to_vec();
        entries.set(KeyRef::String(&fresh, fnv1a(&fresh)), Some(Stored::Number(1.0)), &mut work);
        assert_eq!(work.added, 5);
    }

    /// Two keys that meet in a slot move down as far as their slots stay equal: the key held is hashed again once for the whole
    /// split (Roblox's too: measured), never once a level, and the key added is copied in once. Two keys of one hash go 13 levels
    /// down into a list, the newer first (two colliding keys with 1,024 bytes more of the same after them: a hash of equal state
    /// takes equal bytes to equal states).
    #[test]
    fn a_split_of_two_colliding_keys_hashes_the_held_one_once() {
        let suffix = vec![b'p'; 1024];
        let key = |i: usize| [&colliding(i)[..], &suffix[..]].concat();
        assert_eq!(fnv1a(&key(0)), fnv1a(&key(1)));
        let mut trie: Trie<Rc<[u8]>> = Trie::default();
        let first = key(0);
        trie.insert(&first, fnv1a(&first), Stored::Number(0.0), &mut Work::default());
        let second = key(1);
        let mut work = Work::default();
        trie.insert(&second, fnv1a(&second), Stored::Number(1.0), &mut work);
        assert_eq!((work.rehashed, work.added, work.copied, work.rebuilt), (1072, 1072, 0, 0));
        let mut depth = 0;
        let mut node = trie.root.as_deref().unwrap();
        while let Some(child) = node.nodes.first() {
            node = child;
            depth += 1;
        }
        assert_eq!(depth, 13);
        assert_eq!(node.entries.iter().map(|(k, _)| k.to_vec()).collect::<Vec<_>>(), [second.clone(), first.clone()]);
        // (a key at a slot of its own: nothing hashed again)
        let mut work = Work::default();
        trie.insert(&b"other"[..], fnv1a(b"other"), Stored::Number(2.0), &mut work);
        assert_eq!((work.rehashed, work.added), (0, 5));
    }

    /// A loop walks the entries as they were when it started, in loop order, its own stack never deeper than the trie; a write
    /// while it walks copies its path first, as a write after a shallow copy does, so the loop still walks the value it started on.
    #[test]
    fn a_loop_walks_a_snapshot_in_order_holding_only_a_path() {
        let mut entries = Entries::default();
        let mut strings = FxHashMap::default();
        for n in 0..5000u32 {
            entries.put(Key::Number(n * 7919), Some(Stored::Number(f64::from(n))));
            let key = format!("k{n}").into_bytes();
            entries.put(Key::String(key.as_slice().into()), Some(Stored::Number(f64::from(n))));
            strings.insert(key, f64::from(n));
        }
        let mut numbers = Cursor::new(&entries.numbers);
        let mut walked = Vec::new();
        let mut deepest = 0;
        let mut last = None;
        while let Some((n, value)) = numbers.next() {
            deepest = deepest.max(numbers.stack.len());
            if n == 4999 * 7919 {
                last = Some(value);
            }
            walked.push(n);
            if walked.len() == 10 {
                // (a write while the loop holds the trie: its path copied, the loop's snapshot unchanged)
                let (_, copied) = entries.put(Key::Number(4999 * 7919), Some(Stored::Boolean(true)));
                assert!(copied >= 2, "{copied}");
            }
        }
        assert_eq!(walked, (0..5000).map(|n| n * 7919).collect::<Vec<_>>());
        assert_eq!(number(last.as_ref()), Some(4999.0), "the loop walks the value it started on, not the one written");
        assert!(deepest <= 7, "{deepest}");
        let mut cursor = Cursor::new(&entries.strings);
        let mut keys = Vec::new();
        while let Some((key, _)) = cursor.next() {
            deepest = deepest.max(cursor.stack.len());
            keys.push(key);
        }
        assert!(deepest <= 14 && cursor.stack.is_empty(), "{deepest}");
        let keys: Vec<&[u8]> = keys.iter().map(|key| &**key).collect();
        assert_eq!(keys, string_order(strings.keys().map(Vec::as_slice).collect()));
        assert!(matches!(entries.at(&Key::Number(4999 * 7919)), Some(Stored::Boolean(true))));
    }

    /// A removal and increment's write search a list once: a removal takes the key out at the place its lookup found, and
    /// increment writes at the place its read found (`Entries::locate`, `Entries::replace`), so the host compares no more keys
    /// than the call is charged for. ree searched the list twice and charged one search.
    #[test]
    fn a_removal_and_a_replace_search_a_list_once() {
        let mut trie: Trie<Tiny> = Trie::default();
        for n in 0..50u8 {
            trie.put(Tiny(1, n), Stored::Number(f64::from(n)));
        }
        // (the oldest key is the list's last)
        let oldest = Tiny(1, 0);
        COMPARED.set(0);
        let mut work = Work::default();
        let (_, found) = trie.locate(&oldest, oldest.bits(), &mut work).expect("held");
        let old = trie.insert_at(&oldest, oldest.bits(), Stored::Number(-1.0), found, &mut work);
        assert_eq!((found, COMPARED.get(), work.scanned), (Some(49), 50, 50));
        assert!(matches!(old, Some(Stored::Number(n)) if n == 0.0) && number(trie.at(&oldest)) == Some(-1.0));
        assert_eq!(trie.keys().last(), Some(&oldest), "an overwrite in the table's own list stays in place");
        COMPARED.set(0);
        let mut work = Work::default();
        assert!(trie.remove(&oldest, oldest.bits(), &mut work).is_some());
        assert_eq!((COMPARED.get(), work.scanned, work.rebuilt), (50, 50, 50));
        assert!(trie.at(&oldest).is_none());
        // (one the list does not hold: searched once, nothing taken)
        COMPARED.set(0);
        assert!(trie.take(&Tiny(1, 99)).is_none() && COMPARED.get() == 49 && trie.len == 49);
    }

    /// Letting go of a table's entries frees the entries of the nodes it alone held, which `clear` is charged for: a table's own
    /// all of them, a shallow copy's none, a copy written to only those of the path it copied.
    #[test]
    fn letting_go_counts_the_entries_a_table_alone_held() {
        let mut entries = Entries::default();
        for n in 0..1000u32 {
            entries.put(Key::String(format!("k{n}").into_bytes().into()), Some(Stored::Number(f64::from(n))));
        }
        let copy = entries.clone();
        assert_eq!(release(copy), 0);
        let mut copy = entries.clone();
        copy.put(Key::String(b"k1"[..].into()), Some(Stored::Number(-1.0)));
        let path = release(copy);
        assert!((1..100).contains(&path), "{path}");
        let inner = Shared::new(Entries::default(), false);
        inner.entries.borrow_mut().put(Key::Number(1), Some(Stored::Number(1.0)));
        entries.put(Key::String(b"inner"[..].into()), Some(Stored::Shared(inner)));
        assert_eq!(release(entries), 1002);
    }
}
