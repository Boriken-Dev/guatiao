// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Key/value pairs: the borrowed view, the owned container, and the
//! pair itself.

#![allow(missing_docs)]

use alloc::fmt;
use core::ptr;

use core::marker::PhantomData;

use crate::value::alloc::{Alloc, AllocError, Allocator};
use crate::value::convert::MapError;
use crate::value::error::ValueError;
use crate::value::raw::{dangling, release_buffer, reserve};

use super::{List, Payload, Tag, Text, Value, cloned, only_alloc, or_abort, short_form_alloc};

/// A borrowed sequence of key/value pairs, in **insertion order**, which
/// is part of the contract: consumers render maps as forms, print them
/// as tables and diff them in tests.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Entries<'a> {
    /// First entry. May be null or dangling when `len` is 0.
    ptr: *const Entry,
    /// Number of entries.
    len: usize,
    /// What the view borrows, for the compiler; nothing in C.
    _borrows: PhantomData<&'a [Entry]>,
}

impl<'a> Entries<'a> {
    /// A view of entries this program holds, for as long as it holds it.
    pub const fn new(items: &'a [Entry]) -> Entries<'a> {
        Entries {
            ptr: items.as_ptr(),
            len: items.len(),
            _borrows: PhantomData,
        }
    }

    /// An empty view.
    pub const fn empty() -> Entries<'a> {
        Entries {
            ptr: core::ptr::null(),
            len: 0,
            _borrows: PhantomData,
        }
    }

    /// A view described by hand.
    ///
    /// # Safety
    ///
    /// `len` is 0, or `ptr` addresses `len` initialised entries, their keys
    /// UTF-8, that stay readable and unchanged for `'a`.
    pub const unsafe fn from_raw_parts(ptr: *const Entry, len: usize) -> Entries<'a> {
        Entries {
            ptr,
            len,
            _borrows: PhantomData,
        }
    }

    /// The first element. May be null or dangling when `len` is 0.
    pub const fn ptr(&self) -> *const Entry {
        self.ptr
    }

    /// How many elements.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether it views nothing.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The elements, whatever they hold. A null pointer views nothing.
    const fn items(self) -> &'a [Entry] {
        if self.len == 0 || self.ptr.is_null() {
            return &[];
        }
        // SAFETY: the view was made from a borrow of `'a`, or by
        // `from_raw_parts` or a foreign caller promising the same.
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<'a> From<&'a [Entry]> for Entries<'a> {
    fn from(items: &'a [Entry]) -> Entries<'a> {
        Entries::new(items)
    }
}

/// The entries. An `Entries` made by `new` borrowed entries whose keys
/// are text; one made by `from_raw_parts` was promised so.
impl<'a> From<Entries<'a>> for &'a [Entry] {
    fn from(view: Entries<'a>) -> &'a [Entry] {
        view.items()
    }
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

/// A deep copy of the key and the value.
///
/// # Panics
///
/// As [`Value::clone`], on a value whose content this build refuses.
impl Clone for Entry {
    fn clone(&self) -> Entry {
        Entry {
            key: self.key.clone(),
            value: self.value.clone(),
        }
    }
}

/// The same key, byte for byte, and an equal value.
impl PartialEq for Entry {
    fn eq(&self, other: &Entry) -> bool {
        self.key == other.key && self.value == other.value
    }
}

impl Eq for Entry {}

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
        let this = core::mem::ManuallyDrop::new(self);
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
    pub const fn entries(&self) -> &[Entry] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` entries are initialised.
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Stores `value` under `key`, **consuming** it. Takes anything a
    /// value can be made from, so the common case is one call and no
    /// allocator:
    ///
    /// ```
    /// # use guatiao::Map;
    /// let mut map = Map::new();
    /// map.set("host", "10.0.0.1");
    /// map.set("port", 5900);
    /// map.set("options", Map::new());
    /// ```
    ///
    /// Replacing an existing key keeps its position: moving it to the end
    /// would re-order a caller's rendered form.
    ///
    /// Grows through the allocator this map recorded, or Rust's own for a
    /// map that recorded none, and **aborts** if it refuses, as
    /// `Vec::push` does; [`try_set`](Map::try_set) reports it instead.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        let stored = self.try_set(key, value).map_err(ValueError::from);
        or_abort(stored, core::alloc::Layout::new::<Entry>());
    }

    /// [`set`](Map::set), handing an allocation failure back, for a map
    /// built in an allocator that may refuse: an arena that is full. A
    /// refused store has freed `value`.
    pub fn try_set(&mut self, key: &str, value: impl Into<Value>) -> Result<(), AllocError> {
        let alloc = short_form_alloc(self.alloc());
        self.set_in(key, value, alloc).map_err(only_alloc)
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
        self.set_with(key, alloc, || value)
    }

    /// Stores the node `make` hands over under `key`, calling it only
    /// once nothing can fail: a refused store has not called it, so a
    /// caller moving a node out of its own storage keeps it.
    pub(crate) fn set_with(
        &mut self,
        key: &str,
        alloc: Alloc,
        make: impl FnOnce() -> Value,
    ) -> Result<(), ValueError> {
        match self.position(key) {
            Some(i) => {
                // SAFETY: `i < len`, so this entry is initialised. The
                // replaced value is dropped, which frees what it owned.
                let entry = unsafe { &mut *self.ptr.add(i) };
                drop(core::mem::replace(&mut entry.value, make()));
                Ok(())
            }
            None => self.insert_with(key, alloc, make),
        }
    }

    /// Appends an entry under a key known to be absent. A refused append
    /// frees the node it was handed.
    pub(crate) fn insert_node(
        &mut self,
        key: &str,
        value: Value,
        alloc: Alloc,
    ) -> Result<(), ValueError> {
        self.insert_with(key, alloc, || value)
    }

    /// Appends an entry under a key known to be absent. The key is copied
    /// and the room reserved before `make` is called, so a failure leaves
    /// the map as it was.
    fn insert_with(
        &mut self,
        key: &str,
        alloc: Alloc,
        make: impl FnOnce() -> Value,
    ) -> Result<(), ValueError> {
        let key = Text::new_in(alloc, key)?;
        // SAFETY: the container is consistent.
        unsafe { reserve(self, 1, Some(alloc)) }?;
        // SAFETY: `reserve` guaranteed room for one more entry past `len`,
        // and that slot is uninitialised, so it is written rather than
        // assigned.
        unsafe { self.ptr.add(self.len).write(Entry { key, value: make() }) };
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
    /// map.set("host", "10.0.0.1");
    /// let host: &str = map.required("host")?.try_into()?;
    /// assert_eq!(host, "10.0.0.1");
    /// assert!(map.required("nothing").is_err());
    /// # Ok::<(), Box<dyn core::error::Error>>(())
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
    pub fn iter(&self) -> core::slice::Iter<'_, Entry> {
        self.entries().iter()
    }

    /// Its pairs, in insertion order, each value mutably. See [`IterMut`].
    pub fn iter_mut(&mut self) -> IterMut<'_> {
        IterMut {
            entries: self.entries_mut().iter_mut(),
        }
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

    /// The map `ptr` points at, its keys checked: how a map C hands over
    /// by pointer is read. Its values are checked by their own doors.
    ///
    /// # Safety
    ///
    /// `ptr` points at a map whose storage is consistent and outlives
    /// `'a`. Only the memory is promised: the keys are checked.
    pub const unsafe fn from_ptr<'a>(ptr: *const Map) -> Result<&'a Map, ValueError> {
        // SAFETY: the caller's contract.
        let map = unsafe { &*ptr };
        if map.keys_are_text() {
            Ok(map)
        } else {
            Err(ValueError::NotUtf8)
        }
    }

    /// Whether every key is UTF-8: what a foreign map must show before it
    /// is read as a `Map`. Its values are checked by their own doors.
    pub(crate) const fn keys_are_text(&self) -> bool {
        let entries = self.entries();
        let mut i = 0;
        while i < entries.len() {
            if core::str::from_utf8(entries[i].key_bytes()).is_err() {
                return false;
            }
            i += 1;
        }
        true
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
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
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
    /// storage, writing nothing into the entries: a literal's array may be
    /// read-only. See [`List::dismantle_into`](super::List).
    pub(crate) fn dismantle_into(&mut self, stack: &mut Vec<Value>) {
        for i in 0..self.len {
            // SAFETY: the first `len` entries are initialised, and each is
            // read exactly once -- `len` is zeroed below.
            let Entry { key, value } = unsafe { self.ptr.add(i).read() };
            drop(key);
            stack.push(value);
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
impl core::ops::Index<&str> for Map {
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

/// The pairs of a map, in order, as `HashMap::iter_mut` gives them: the
/// key shared and the value mutable, so a key cannot change in place and
/// move a value to a key nobody searches for, or repeat another.
///
/// ```compile_fail
/// use guatiao::{Entry, Map, Text, Value};
/// let mut map = Map::new();
/// map.set("a", 1);
/// for entry in &mut map {
///     *entry = Entry::new(Text::new("b"), Value::from(2));
/// }
/// ```
#[derive(Debug)]
pub struct IterMut<'a> {
    entries: core::slice::IterMut<'a, Entry>,
}

impl<'a> Iterator for IterMut<'a> {
    type Item = (&'a Text, &'a mut Value);

    fn next(&mut self) -> Option<(&'a Text, &'a mut Value)> {
        self.entries.next().map(|e| (&e.key, &mut e.value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl DoubleEndedIterator for IterMut<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.next_back().map(|e| (&e.key, &mut e.value))
    }
}

impl ExactSizeIterator for IterMut<'_> {}

impl<'a> IntoIterator for &'a mut Map {
    type Item = (&'a Text, &'a mut Value);
    type IntoIter = IterMut<'a>;

    fn into_iter(self) -> IterMut<'a> {
        self.iter_mut()
    }
}

impl<'a> IntoIterator for &'a Map {
    type Item = &'a Entry;
    type IntoIter = core::slice::Iter<'a, Entry>;

    fn into_iter(self) -> core::slice::Iter<'a, Entry> {
        self.iter()
    }
}

impl Clone for Map {
    /// A deep copy into this binary's allocator, as [`Value::clone`]
    /// makes one; `clone_in(alloc)` names another.
    ///
    /// # Panics
    ///
    /// As [`Value::clone`]: on content this build refuses, which only a
    /// foreign producer can write. [`clone_in`](Map::clone_in) reports it.
    fn clone(&self) -> Map {
        cloned(
            self.clone_in(Alloc::rust()),
            core::alloc::Layout::new::<Entry>(),
        )
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

/// Sets each pair as [`Map::set`] does, aborting if the allocator
/// refuses.
impl<K: AsRef<str>, V: Into<Value>> Extend<(K, V)> for Map {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        for (key, value) in iter {
            self.set(key.as_ref(), value);
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
    pub(crate) const fn key_bytes(&self) -> &[u8] {
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
