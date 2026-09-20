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

use super::{List, Payload, Tag, Text, Value};

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
    /// The key. Raw bytes: `"Host"` and `"host"` are two keys.
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
    /// As [`List::from_raw_parts`](super::List::from_raw_parts).
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
    fn insert_node(&mut self, key: &str, value: Value, alloc: Alloc) -> Result<(), ValueError> {
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
        self.entries()
            .iter()
            .position(|e| e.key() == key.as_bytes())
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

    /// Its keys, in insertion order, as raw bytes.
    pub fn keys(&self) -> impl Iterator<Item = &[u8]> {
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
            match key.as_str() {
                Some(key) => match self.set_node(key, value, alloc) {
                    Ok(()) => n += 1,
                    Err(e) => result = Err(e),
                },
                None => result = Err(ValueError::NotUtf8),
            }
            if result.is_err() {
                break;
            }
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

    /// A deep copy through `alloc`, bounded by [`MAX_DEPTH`](crate::MAX_DEPTH).
    pub fn clone_in(&self, alloc: Alloc) -> Result<Map, ValueError> {
        self.clone_at(alloc, 0)
    }

    pub(crate) fn clone_at(&self, alloc: Alloc, depth: u32) -> Result<Map, ValueError> {
        let entries = self.entries();
        // The guard owns everything built so far, so an early return frees
        // it rather than leaking it.
        let mut out = Map::new_in(alloc);
        if !entries.is_empty() {
            // SAFETY: the container is consistent and empty.
            unsafe { reserve(&mut out, entries.len(), Some(alloc))? };
        }
        for entry in entries {
            let key = entry.key_str().ok_or(ValueError::NotUtf8)?;
            let value = entry.value.clone_at(alloc, depth + 1)?;
            out.insert_node(key, value, alloc)?;
        }
        Ok(out)
    }

    /// Pairwise, in order, each value through [`Value`]'s own
    /// comparison. **Order is significant**: two maps with the same pairs
    /// in a different order are two different values.
    pub(crate) fn eq_at(&self, other: &Map, depth: u32) -> bool {
        let (x, y) = (self.entries(), other.entries());
        x.len() == y.len()
            && x.iter()
                .zip(y)
                .all(|(p, q)| p.key() == q.key() && p.value.eq_at(&q.value, depth + 1))
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
    /// Structural, as [`Value`]'s is.
    fn eq(&self, other: &Map) -> bool {
        self.eq_at(other, 0)
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

    /// The key, as bytes: a key may contain a NUL, and comparing only to
    /// the first would make two different keys look identical.
    /// [`key_str`](Entry::key_str) is the checked reading.
    pub fn key(&self) -> &[u8] {
        self.key.as_bytes()
    }

    /// The key as text, or `None` if it is not valid UTF-8.
    pub fn key_str(&self) -> Option<&str> {
        std::str::from_utf8(self.key()).ok()
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
