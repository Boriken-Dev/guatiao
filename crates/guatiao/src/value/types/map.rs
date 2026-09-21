// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Key/value pairs: the borrowed view, the owned container, and the
//! pair itself.

#![allow(missing_docs)]

use std::fmt;
use std::ptr;

use crate::value::alloc::{Alloc, Allocator};
use crate::value::convert::MapError;
use crate::value::error::ValueError;
use crate::value::raw::{dangling, release_buffer, reserve};

use super::{List, Payload, Tag, Text, Value, or_abort};

/// A borrowed sequence of key/value pairs, in **insertion order**, which
/// is part of the contract: consumers render maps as forms, print them
/// as tables and diff them in tests.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Entries {
    /// First entry. May be null or dangling when `len` is 0.
    pub ptr: *const Entry,
    /// Number of entries.
    pub len: usize,
}

/// An owned, growable sequence of key/value pairs, in insertion order.
///
/// Lookup is a linear scan by contract: this is a metadata container of
/// tens of keys, and at that size a scan over contiguous memory beats
/// hashing every lookup key. Setting an existing key replaces it **in
/// place**, keeping its position.
#[repr(C)]
#[derive(Debug)]
pub struct Map {
    /// First entry.
    pub(crate) ptr: *mut Entry,
    /// Number of entries.
    pub(crate) len: usize,
    /// Capacity in entries. 0 means the buffer is not owned.
    pub(crate) cap: usize,
    /// The allocator that made this buffer. Null when `cap == 0`.
    pub(crate) alloc: *const Allocator,
}

/// One key/value pair. The value is held **inline**, not by pointer, so
/// a map is one contiguous array.
#[repr(C)]
pub struct Entry {
    /// The key, compared by bytes: `"Host"` and `"host"` are two keys.
    pub(crate) key: Text,
    /// The value.
    pub(crate) value: Value,
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("key_len", &self.key.len)
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        for i in 0..self.len {
            // SAFETY: as for `List`.
            let entry = unsafe { self.ptr.add(i).read() };
            // The entry's own glue releases the key and frees the value.
            drop(entry);
        }
        self.len = 0;
        // SAFETY: the entries have been moved out.
        unsafe { release_buffer(self) }
    }
}

impl Map {
    /// A map over storage described by hand, the elements being entries.
    ///
    /// # Safety
    ///
    /// As [`List::from_raw_parts`](super::List::from_raw_parts), and
    /// every key is UTF-8, as a [`Text`]'s bytes are.
    pub unsafe fn from_raw_parts(
        ptr: *mut Entry,
        len: usize,
        cap: usize,
        alloc: *const Allocator,
    ) -> Map {
        Map {
            ptr,
            len,
            cap,
            alloc,
        }
    }

    /// The four fields, with ownership: this map no longer frees them.
    pub fn into_raw_parts(self) -> (*mut Entry, usize, usize, *const Allocator) {
        let this = std::mem::ManuallyDrop::new(self);
        (this.ptr, this.len, this.cap, this.alloc)
    }

    /// How many entries the storage holds before it must grow.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// An empty map **container**: grow it with [`set`](Map::set), and
    /// `map.into()` makes it a [`Value`] where one is wanted. Creating it
    /// allocates nothing and it frees what it grew into on drop.
    pub fn new() -> Map {
        Map::new_in(Alloc::rust())
    }

    /// The same, growing through an allocator you name.
    pub fn new_in(alloc: Alloc) -> Map {
        Map {
            ptr: dangling::<Entry>(),
            len: 0,
            cap: 0,
            alloc: alloc.as_raw(),
        }
    }

    /// The allocator this map grows through.
    pub fn alloc(&self) -> Result<Alloc, ValueError> {
        // SAFETY: the address a container recorded is an allocator that
        // outlives it, by the contract on `Alloc`.
        Ok(unsafe { Alloc::from_raw(self.alloc) }?)
    }

    /// An empty map with room for `capacity` entries, through `alloc`.
    pub(crate) fn with_capacity_in(alloc: Alloc, capacity: usize) -> Result<Map, ValueError> {
        let mut map = Map::new_in(alloc);
        if capacity > 0 {
            // SAFETY: the container is consistent and empty.
            unsafe { reserve(&mut map, capacity, Some(alloc))? };
        }
        Ok(map)
    }

    /// How many entries it holds.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Its entries, in insertion order, which is part of the contract.
    pub fn entries(&self) -> &[Entry] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` entries are initialised.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Stores `value` under `key`, **consuming** it. Takes anything a
    /// value can be made from, so the common case is one call and no
    /// allocator:
    ///
    /// ```
    /// # use guatiao::Map;
    /// let mut map = Map::new();
    /// map.set("host", "10.0.0.1")?;
    /// map.set("port", 5900)?;
    /// map.set("options", Map::new())?;
    /// # Ok::<(), guatiao::ValueError>(())
    /// ```
    ///
    /// Replacing an existing key keeps its position: moving it to the end
    /// would re-order a caller's rendered form.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) -> Result<(), ValueError> {
        let alloc = self.alloc()?;
        self.set_in(key, value, alloc)
    }

    /// The same, adopting `alloc` for a map that carries none.
    pub fn set_in(
        &mut self,
        key: &str,
        value: impl Into<Value>,
        alloc: Alloc,
    ) -> Result<(), ValueError> {
        self.set_node(key, value.into(), alloc)
    }

    /// Stores an already-built node. A refused store frees it: the node
    /// was moved in, so nothing else can.
    fn set_node(&mut self, key: &str, value: Value, alloc: Alloc) -> Result<(), ValueError> {
        match self.position(key) {
            Some(i) => {
                // SAFETY: `i < len`, so this entry is initialised. The
                // replaced value is dropped, which frees what it owned.
                let entry = unsafe { &mut *self.ptr.add(i) };
                drop(std::mem::replace(&mut entry.value, value));
                Ok(())
            }
            None => self.insert_node(key, value, alloc),
        }
    }

    /// Appends an entry under a key known to be absent. The key copy is
    /// made first, so a failure there leaves the map as it was.
    pub(crate) fn insert_node(
        &mut self,
        key: &str,
        value: Value,
        alloc: Alloc,
    ) -> Result<(), ValueError> {
        let key_owned = match Text::new_in(alloc, key) {
            Ok(k) => k,
            Err(e) => {
                drop(value);
                return Err(e);
            }
        };
        // SAFETY: the container is consistent.
        if let Err(e) = unsafe { reserve(self, 1, Some(alloc)) } {
            drop((key_owned, value));
            return Err(e.into());
        }
        // SAFETY: `reserve` guaranteed room for one more entry past `len`,
        // and that slot is uninitialised, so it is written rather than
        // assigned.
        unsafe {
            self.ptr.add(self.len).write(Entry {
                key: key_owned,
                value,
            })
        };
        self.len += 1;
        Ok(())
    }

    /// The position of `key`, by exact byte comparison -- not `strcmp`,
    /// since a key may contain a NUL.
    fn position(&self, key: &str) -> Option<usize> {
        self.entries().iter().position(|e| e.key() == key)
    }

    /// The value under `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        let i = self.position(key)?;
        self.entries().get(i).map(|e| &e.value)
    }

    /// The value under `key`, mutably.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        let i = self.position(key)?;
        // SAFETY: `i < len`, so this entry is initialised.
        Some(unsafe { &mut (*self.ptr.add(i)).value })
    }

    /// The value under `key`, or [`MapError::MissingKey`] **naming it**:
    /// the step from a lookup to a value, so a read is one expression.
    ///
    /// ```
    /// # use guatiao::Map;
    /// let mut map = Map::new();
    /// map.set("host", "10.0.0.1")?;
    /// let host: &str = map.required("host")?.try_into()?;
    /// assert_eq!(host, "10.0.0.1");
    /// assert!(map.required("nothing").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn required(&self, key: &str) -> Result<&Value, MapError> {
        self.get(key).ok_or_else(|| MapError::missing(key))
    }

    /// Whether `key` is present.
    pub fn contains_key(&self, key: &str) -> bool {
        self.position(key).is_some()
    }

    /// Appends `value` to the list under `key`, creating the list when
    /// there is none.
    pub fn push_into(&mut self, key: &str, value: impl Into<Value>) -> Result<(), ValueError> {
        let alloc = self.alloc()?;
        if !self.contains_key(key) {
            // A LIST, not a map. The key names a sequence being appended
            // to.
            self.set_in(key, List::new_in(alloc), alloc)?;
        }
        let node = self.get_mut(key).ok_or(ValueError::WrongKind)?;
        let list: &mut List =
            crate::value::convert::TryAsMut::try_as_mut(node).ok_or(ValueError::WrongKind)?;
        list.push_in(value, alloc)
    }

    /// Removes `key` and hands back its value, keeping the order of the
    /// rest.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let i = self.position(key)?;
        // SAFETY: `i < len`, so the entry is initialised.
        let entry = unsafe { self.ptr.add(i).read() };
        // SAFETY: closing the hole the removed entry left.
        unsafe { ptr::copy(self.ptr.add(i + 1), self.ptr.add(i), self.len - i - 1) };
        self.len -= 1;
        let (_key, value) = entry.into_parts();
        Some(value)
    }

    /// Removes `key` and frees its value. Answers whether it was there.
    pub fn discard(&mut self, key: &str) -> bool {
        self.remove(key).is_some()
    }

    /// Frees every entry, **keeping the capacity** already paid for.
    pub fn clear(&mut self) {
        for i in 0..self.len {
            // SAFETY: the first `len` entries are initialised, and each is
            // read exactly once -- `len` is zeroed below.
            drop(unsafe { self.ptr.add(i).read() });
        }
        self.len = 0;
    }

    /// Its entries, in insertion order.
    pub fn iter(&self) -> std::slice::Iter<'_, Entry> {
        self.entries().iter()
    }

    /// Its keys, in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.iter().map(Entry::key)
    }

    /// Its values, in insertion order.
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.iter().map(Entry::value)
    }

    /// Copies every entry of `src` into this map, replacing keys that
    /// collide and appending the rest, and answers how many.
    ///
    /// **Use this before rebuilding a record**, or every field you do not
    /// model is dropped on write-back. **Not atomic**: a failure at entry
    /// *k* leaves `0..k` applied, leaking nothing.
    pub fn copy_from(&mut self, src: &Map) -> Result<usize, ValueError> {
        let alloc = self.alloc()?;
        self.copy_from_in(src, alloc)
    }

    /// The same, adopting `alloc` for a map that carries none.
    pub fn copy_from_in(&mut self, src: &Map, alloc: Alloc) -> Result<usize, ValueError> {
        self.absorb(src.clone_in(alloc)?, alloc)
    }

    /// Moves every entry of `from` into this map. Each MOVES out and
    /// `from`'s length falls with it, so whatever is left when this stops
    /// is freed with `from` exactly once.
    pub(crate) fn absorb(&mut self, from: Map, alloc: Alloc) -> Result<usize, ValueError> {
        let mut from = from;
        let mut n = 0;
        let mut result = Ok(());
        while let Some(entry) = from.take_first() {
            let (key, value) = entry.into_parts();
            result = self.set_node(&key, value, alloc);
            if result.is_err() {
                break;
            }
            n += 1;
        }
        result?;
        Ok(n)
    }

    /// Removes the first entry, keeping the order of the rest.
    fn take_first(&mut self) -> Option<Entry> {
        if self.len == 0 {
            return None;
        }
        // SAFETY: the first entry is initialised.
        let out = unsafe { self.ptr.read() };
        // SAFETY: closing the hole it left. At `len == 1` the source is a
        // legal one-past-the-end pointer and the count is zero.
        unsafe { ptr::copy(self.ptr.add(1), self.ptr, self.len - 1) };
        self.len -= 1;
        Some(out)
    }

    /// Whether every key is UTF-8: what a foreign map must show before it
    /// is read as a `Map`. Its values are checked by their own doors.
    pub(crate) fn keys_are_text(&self) -> bool {
        self.entries()
            .iter()
            .all(|entry| std::str::from_utf8(entry.key_bytes()).is_ok())
    }

    /// A deep copy through `alloc`, at any depth.
    pub fn clone_in(&self, alloc: Alloc) -> Result<Map, ValueError> {
        crate::value::walk::copy_map(self, alloc)
    }

    /// The entries, mutably, for reordering them in place. Private: a
    /// caller changing a key would move a value to a key nobody searched
    /// for.
    fn entries_mut(&mut self) -> &mut [Entry] {
        if self.len == 0 {
            return &mut [];
        }
        // SAFETY: the first `len` entries are initialised, and `&mut self`
        // makes the borrow unique.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    /// Removes the last entry.
    fn pop_last(&mut self) -> Option<Entry> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        // SAFETY: the entry at the old last index is initialised, and with
        // `len` lowered nothing reads it again.
        Some(unsafe { self.ptr.add(self.len).read() })
    }

    /// Frees every key, moves every value onto `stack` and frees the
    /// storage. See [`List::dismantle_into`](super::List).
    pub(crate) fn dismantle_into(&mut self, stack: &mut Vec<Value>) {
        for i in 0..self.len {
            // SAFETY: the first `len` entries are initialised.
            let entry = unsafe { &mut *self.ptr.add(i) };
            entry.key.release();
            stack.push(std::mem::take(&mut entry.value));
        }
        self.len = 0;
        // SAFETY: the entries have been moved out.
        unsafe { release_buffer(self) }
    }
}

/// `map["host"]`, as `HashMap` gives it: panics when nothing is stored
/// under the key. There is no `IndexMut`, for the same reason `HashMap`
/// has none: it could only hand back a slot that already exists, so
/// `map["new"] = v` would panic rather than insert. [`Map::set`] inserts.
impl std::ops::Index<&str> for Map {
    type Output = Value;

    fn index(&self, key: &str) -> &Value {
        match self.get(key) {
            Some(value) => value,
            None => panic!("no value under `{key}`"),
        }
    }
}

/// The entries, moved out in order as `(key, value)`. What is not taken is
/// freed with the iterator.
#[derive(Debug)]
pub struct IntoIter {
    /// The rest, in reverse, so each `next` is a `pop_last`.
    rest: Map,
}

impl Iterator for IntoIter {
    type Item = (Text, Value);

    fn next(&mut self) -> Option<(Text, Value)> {
        self.rest.pop_last().map(Entry::into_parts)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.rest.len(), Some(self.rest.len()))
    }
}

impl ExactSizeIterator for IntoIter {}

impl IntoIterator for Map {
    type Item = (Text, Value);
    type IntoIter = IntoIter;

    fn into_iter(mut self) -> IntoIter {
        self.entries_mut().reverse();
        IntoIter { rest: self }
    }
}

/// Each entry, mutably. [`Entry::value_mut`] reaches the value; the key
/// stays the key.
impl<'a> IntoIterator for &'a mut Map {
    type Item = &'a mut Entry;
    type IntoIter = std::slice::IterMut<'a, Entry>;

    fn into_iter(self) -> std::slice::IterMut<'a, Entry> {
        self.entries_mut().iter_mut()
    }
}

impl<'a> IntoIterator for &'a Map {
    type Item = &'a Entry;
    type IntoIter = std::slice::Iter<'a, Entry>;

    fn into_iter(self) -> std::slice::Iter<'a, Entry> {
        self.iter()
    }
}

impl Clone for Map {
    /// A deep copy through the allocator this container recorded, or the
    /// crate's own when it has none yet. Panics as [`Value::clone`] does.
    fn clone(&self) -> Map {
        let alloc = Alloc::recorded_or_rust(self.alloc);
        self.clone_in(alloc)
            .expect("a well-formed container clones through a working allocator")
    }
}

impl PartialEq for Map {
    /// Pairwise, in order, each value by [`Value`]'s own comparison.
    /// **Order is significant**: two maps with the same pairs in a
    /// different order are two different values.
    fn eq(&self, other: &Map) -> bool {
        crate::value::walk::equal(crate::value::walk::Pair::Maps(self, other))
    }
}

impl Eq for Map {}

// SAFETY: the buffer is owned outright and reached only through `&self`
// or `&mut self`; the allocator it recorded outlives it and may be
// called from any thread, which is the contract on `Allocator`.
unsafe impl Send for Map {}
// SAFETY: as above.
unsafe impl Sync for Map {}

impl Default for Map {
    fn default() -> Map {
        Map::new()
    }
}

/// Collects `(key, value)` pairs, as `HashMap` does, but in insertion
/// order; a repeated key replaces the value in place. Grows through Rust's
/// allocator and aborts if it refuses.
impl<K: AsRef<str>, V: Into<Value>> FromIterator<(K, V)> for Map {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Map {
        let mut map = Map::new();
        map.extend(iter);
        map
    }
}

/// Sets each pair through the allocator this map recorded. Panics if it
/// refuses, as [`List`]'s `Extend` does.
impl<K: AsRef<str>, V: Into<Value>> Extend<(K, V)> for Map {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        for (key, value) in iter {
            or_abort(self.set(key.as_ref(), value));
        }
    }
}

impl From<Map> for Value {
    /// Safe for a map holding anything, because it **moves**: the
    /// container is consumed, the node takes over its buffer, and there
    /// is never a moment when two structs describe one allocation.
    fn from(map: Map) -> Value {
        let mut v = Value::blank(Tag::GUATIAO_MAP);
        v.payload = Payload::map(map);
        v
    }
}

impl Entry {
    /// An entry from an owned key and an owned value.
    pub fn new(key: Text, value: Value) -> Entry {
        Entry { key, value }
    }

    /// The key and the value, with ownership.
    pub fn into_parts(self) -> (Text, Value) {
        (self.key, self.value)
    }

    /// The key. It may contain a NUL, so a C reader compares its length
    /// too.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The key's stored bytes, for comparing a map not yet checked.
    pub(crate) fn key_bytes(&self) -> &[u8] {
        self.key.bytes()
    }

    /// The value stored under it.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The value stored under it, mutably. The key stays the key:
    /// changing one in place would move a value to a key nobody searched
    /// for. Use `remove` and `set` for that.
    pub fn value_mut(&mut self) -> &mut Value {
        &mut self.value
    }
}
