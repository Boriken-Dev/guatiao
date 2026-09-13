// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Key/value pairs: the borrowed view, the owned container, and the
//! pair itself.

#![allow(missing_docs)]

use std::fmt;
use std::mem::ManuallyDrop;
use std::ptr;

use crate::value::alloc::Allocator;
// The raw layer this crate keeps to itself: the free walk, the
// allocator-taking mutators and the private helpers. Imported whole
// because the impl below calls into it at almost every line.
use crate::value::alloc::Alloc;
use crate::value::mutate::*;
use crate::value::raw::{dangling, release_buffer};

use super::{Payload, Tag, Text, Value};

/// A borrowed sequence of key/value pairs, in **insertion order**.
///
/// Order is part of the contract, not an artefact: consumers render maps
/// as forms, print them as tables and diff them in tests, and all three
/// need it stable and meaningful.
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
/// Lookup is a linear scan, by contract rather than by accident: this is a
/// metadata container holding tens of keys, and at that size a scan over
/// contiguous memory beats hashing every lookup key. Setting an existing
/// key replaces it **in place**, keeping its position.
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

/// One key/value pair of a map. The value is held **inline**, not by
/// pointer, so a map is one contiguous array.
#[repr(C)]
pub struct Entry {
    /// The key. Raw bytes: no case folding, no normalisation, no trimming.
    /// `"Host"` and `"host"` are two keys.
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
    /// A map over storage described by hand. See
    /// [`List::from_raw_parts`](super::List::from_raw_parts); the elements
    /// are entries.
    ///
    /// # Safety
    ///
    /// As `List::from_raw_parts`, over entries.
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

    /// How many entries the storage holds before it must grow. Zero for
    /// an array this map does not own.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// An empty map **container**, growing through Rust's allocator.
    ///
    /// A container rather than a value: grow it with [`set`](Map::set), and
    /// `map.into()` makes it a [`Value`] at the point something wants one.
    /// Creating it allocates nothing — `cap == 0` never reaches an
    /// allocator — and it frees whatever it grew into on drop.
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

    /// Stores `value` under `key`, **consuming** it.
    ///
    /// Takes anything a value can be made from, so the common case is one
    /// call and no allocator:
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
    /// Replacing an existing key keeps its position, because the ordinary
    /// use is "build a map, then override two fields" and moving a
    /// replaced key to the end would re-order a caller's rendered form.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) -> Result<(), ValueError> {
        let alloc = self.alloc()?;
        let mut value = value.into();
        self.as_node(|node| {
            // SAFETY: `node` is this map, and `value` is a well-formed
            // tree consumed here, so nothing else refers to what it owned.
            unsafe { map_set(node, key, &mut value, alloc) }
        })
    }

    /// The value under `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        let i = position_in(self.entries(), key)?;
        self.entries().get(i).map(|e| &e.value)
    }

    /// The value under `key`, mutably.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        let i = position_in(self.entries(), key)?;
        // SAFETY: `i < len`, so this entry is initialised.
        Some(unsafe { &mut (*self.ptr.add(i)).value })
    }

    /// Whether `key` is present.
    pub fn contains_key(&self, key: &str) -> bool {
        position_in(self.entries(), key).is_some()
    }

    /// Removes `key` and hands back its value, keeping the order of the
    /// rest. The value frees itself when it goes out of scope.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        // SAFETY: `self` is a well-formed map by construction.
        self.as_node(|node| unsafe { map_remove(node, key) })
    }

    /// Frees every entry, **keeping the capacity** already paid for.
    pub fn clear(&mut self) {
        // SAFETY: as above.
        let _ = self.as_node(|node| unsafe { map_clear(node) });
    }

    /// Runs `body` with this container seen as the node it would be.
    ///
    /// **Why the detour.** Every mutation has to check a tag before it
    /// touches an arm, so the operations are written against [`Value`];
    /// a `&mut Map` cannot become a `&mut Value`, because the map sits at
    /// an offset inside the node rather than at its start.
    ///
    /// So the map MOVES into a node, the node is operated on, and the map
    /// moves back. Every step is a move, so nothing is ever described by
    /// two containers at once — which is the mistake that corrupts a
    /// heap here. The placeholder left behind for the duration owns
    /// nothing, so a panic inside `body` frees the tree exactly once and
    /// leaves this container empty rather than dangling.
    fn as_node<R>(&mut self, body: impl FnOnce(&mut Value) -> R) -> R {
        let empty = Map {
            ptr: dangling::<Entry>(),
            len: 0,
            cap: 0,
            alloc: self.alloc,
        };
        let mut node = Value::from(std::mem::replace(self, empty));
        let out = body(&mut node);
        let node = ManuallyDrop::new(node);
        // SAFETY: the node was built from a map immediately above and no
        // operation changes a node's kind, so the map arm is live. The
        // read moves it out, and the node is not dropped.
        *self = unsafe { ptr::read(&*node.payload.map) };
        out
    }
}

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
        let mut v = blank(Tag::GUATIAO_MAP);
        v.payload = Payload {
            map: ManuallyDrop::new(map),
        };
        v
    }
}

impl Entry {
    /// An entry from an owned key and an owned value. Safe: both are
    /// whole, and the entry now owns them.
    pub fn new(key: Text, value: Value) -> Entry {
        Entry { key, value }
    }

    /// The key and the value, with ownership.
    pub fn into_parts(self) -> (Text, Value) {
        (self.key, self.value)
    }

    /// The key, as bytes.
    ///
    /// Bytes rather than text, because a key may contain a NUL and
    /// comparing only to the first one would make two different keys look
    /// identical. [`key_str`](Entry::key_str) is the checked reading.
    pub fn key(&self) -> &[u8] {
        key_bytes(self)
    }

    /// The key as text, or `None` if it is not valid UTF-8.
    pub fn key_str(&self) -> Option<&str> {
        std::str::from_utf8(self.key()).ok()
    }

    /// The value stored under it.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The value stored under it, mutably.
    ///
    /// The key stays the key: a map is insertion-ordered and looked up by
    /// exact bytes, so changing one in place would move a value to a key
    /// nobody searched for. Use `remove` and `set` for that.
    pub fn value_mut(&mut self) -> &mut Value {
        &mut self.value
    }
}
