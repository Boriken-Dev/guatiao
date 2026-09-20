// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A sequence of values: the borrowed view and the owned container.

#![allow(missing_docs)]

use std::ptr;

use crate::value::alloc::{Alloc, Allocator};
use crate::value::error::ValueError;
use crate::value::raw::{dangling, release_buffer, reserve};

use super::{Payload, Tag, Value};

/// A borrowed sequence of values, in order.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Values {
    /// First element. May be null or dangling when `len` is 0.
    pub ptr: *const Value,
    /// Number of elements.
    pub len: usize,
}

/// An owned, growable sequence of values.
#[repr(C)]
#[derive(Debug)]
pub struct List {
    /// First element.
    pub(crate) ptr: *mut Value,
    /// Number of elements.
    pub(crate) len: usize,
    /// Capacity in elements. 0 means the buffer is not owned.
    pub(crate) cap: usize,
    /// The allocator that made this buffer. Null when `cap == 0`.
    pub(crate) alloc: *const Allocator,
}

impl Drop for List {
    fn drop(&mut self) {
        for i in 0..self.len {
            // SAFETY: the first `len` elements are initialised, and
            // each is read exactly once -- `len` is zeroed below.
            //
            // Dropping one enters `Value::drop`'s ITERATIVE walk, so the
            // recursion stops here at depth one.
            drop(unsafe { self.ptr.add(i).read() });
        }
        self.len = 0;
        // SAFETY: the elements have been moved out.
        unsafe { release_buffer(self) }
    }
}

impl List {
    /// A list over storage described by hand, the elements being nodes.
    ///
    /// # Safety
    ///
    /// The first `len` nodes at `ptr` are well formed and readable for as
    /// long as this lives; `cap > 0` means the block came from `alloc`
    /// with a layout of `cap` nodes and is this list's alone; `cap == 0`
    /// means the array is somebody else's and, if mutated in place,
    /// writable.
    pub unsafe fn from_raw_parts(
        ptr: *mut Value,
        len: usize,
        cap: usize,
        alloc: *const Allocator,
    ) -> List {
        List {
            ptr,
            len,
            cap,
            alloc,
        }
    }

    /// The four fields, with ownership: this list no longer frees them.
    pub fn into_raw_parts(self) -> (*mut Value, usize, usize, *const Allocator) {
        let this = std::mem::ManuallyDrop::new(self);
        (this.ptr, this.len, this.cap, this.alloc)
    }

    /// How many nodes the storage holds before it must grow. Zero for an
    /// array this list does not own.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// An empty list **container**. See [`Map::new`](super::Map::new).
    pub fn new() -> List {
        List::new_in(Alloc::rust())
    }

    /// The same, growing through an allocator you name.
    pub fn new_in(alloc: Alloc) -> List {
        List {
            ptr: dangling::<Value>(),
            len: 0,
            cap: 0,
            alloc: alloc.as_raw(),
        }
    }

    /// The allocator this list grows through.
    pub fn alloc(&self) -> Result<Alloc, ValueError> {
        // SAFETY: as for [`Map::alloc`].
        Ok(unsafe { Alloc::from_raw(self.alloc) }?)
    }

    /// How many elements it holds.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Its elements, in order.
    pub fn items(&self) -> &[Value] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` elements are initialised.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Appends `value`, **consuming** it. Takes anything [`Map::set`](super::Map::set)
    /// does.
    pub fn push(&mut self, value: impl Into<Value>) -> Result<(), ValueError> {
        let alloc = self.alloc()?;
        self.push_in(value, alloc)
    }

    /// The same, adopting `alloc` for a list that carries none.
    pub fn push_in(&mut self, value: impl Into<Value>, alloc: Alloc) -> Result<(), ValueError> {
        self.push_node(value.into(), alloc)
    }

    /// Appends an already-built node. A refused append frees what it was
    /// handed: it was moved in, so nothing else can.
    fn push_node(&mut self, value: Value, alloc: Alloc) -> Result<(), ValueError> {
        // SAFETY: the container is consistent.
        if let Err(e) = unsafe { reserve(self, 1, Some(alloc)) } {
            drop(value);
            return Err(e.into());
        }
        // SAFETY: `reserve` guaranteed room for one more element past
        // `len`, and that slot is uninitialised, so it is written rather
        // than assigned.
        unsafe { self.ptr.add(self.len).write(value) };
        self.len += 1;
        Ok(())
    }

    /// The element at `index`.
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.items().get(index)
    }

    /// The element at `index`, mutably.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Value> {
        if index >= self.len {
            return None;
        }
        // SAFETY: `index < len`, so the element is initialised.
        Some(unsafe { &mut *self.ptr.add(index) })
    }

    /// Its elements, in order.
    pub fn iter(&self) -> std::slice::Iter<'_, Value> {
        self.items().iter()
    }

    /// Removes the element at `index`, keeping the order of the rest.
    /// The returned node **owns its buffers**, so dropping it on the
    /// floor is a release rather than a leak.
    pub fn remove(&mut self, index: usize) -> Option<Value> {
        if index >= self.len {
            return None;
        }
        // SAFETY: `index < len`, so this element is initialised; the shift
        // below closes the hole it leaves.
        let out = unsafe { self.ptr.add(index).read() };
        // SAFETY: moving the tail down one slot over the hole just vacated.
        unsafe {
            ptr::copy(
                self.ptr.add(index + 1),
                self.ptr.add(index),
                self.len - index - 1,
            )
        };
        self.len -= 1;
        Some(out)
    }

    /// Removes and frees the element at `index`. Answers whether there was
    /// one.
    pub fn discard(&mut self, index: usize) -> bool {
        self.remove(index).is_some()
    }

    /// Removes the last element, as `Vec::pop` does.
    pub fn pop(&mut self) -> Option<Value> {
        self.remove(self.len.checked_sub(1)?)
    }

    /// Frees every element, **keeping the capacity** already paid for.
    pub fn clear(&mut self) {
        for i in 0..self.len {
            // SAFETY: the first `len` elements are initialised, and each
            // is read exactly once -- `len` is zeroed below.
            drop(unsafe { self.ptr.add(i).read() });
        }
        self.len = 0;
    }

    /// A deep copy through `alloc`, bounded by [`MAX_DEPTH`](crate::MAX_DEPTH).
    pub fn clone_in(&self, alloc: Alloc) -> Result<List, ValueError> {
        self.clone_at(alloc, 0)
    }

    pub(crate) fn clone_at(&self, alloc: Alloc, depth: u32) -> Result<List, ValueError> {
        let items = self.items();
        // The guard owns everything built so far, so an early return frees
        // it rather than leaking it.
        let mut out = List::new_in(alloc);
        if !items.is_empty() {
            // SAFETY: the container is consistent and empty.
            unsafe { reserve(&mut out, items.len(), Some(alloc))? };
        }
        for item in items {
            out.push_node(item.clone_at(alloc, depth + 1)?, alloc)?;
        }
        Ok(out)
    }

    /// Element by element, each through [`Value`]'s own comparison.
    /// Moves every element onto `stack` and frees the storage, so a
    /// value's drop takes a tree apart without recursing.
    pub(crate) fn dismantle_into(&mut self, stack: &mut Vec<Value>) {
        for i in 0..self.len {
            // SAFETY: the first `len` elements are initialised.
            stack.push(std::mem::take(unsafe { &mut *self.ptr.add(i) }));
        }
        self.len = 0;
        // SAFETY: the elements have been moved out.
        unsafe { release_buffer(self) }
    }

    pub(crate) fn eq_at(&self, other: &List, depth: u32) -> bool {
        let (x, y) = (self.items(), other.items());
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.eq_at(q, depth + 1))
    }
}

impl<'a> IntoIterator for &'a List {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;

    fn into_iter(self) -> std::slice::Iter<'a, Value> {
        self.iter()
    }
}

impl Clone for List {
    /// A deep copy through the allocator this container recorded, or the
    /// crate's own when it has none yet. Panics as [`Value::clone`] does.
    fn clone(&self) -> List {
        let alloc = Alloc::recorded_or_rust(self.alloc);
        self.clone_in(alloc)
            .expect("a well-formed container clones through a working allocator")
    }
}

impl PartialEq for List {
    /// Structural, and bounded as [`Value`]'s is: two trees nested
    /// deeper than [`MAX_DEPTH`](crate::MAX_DEPTH) compare unequal.
    fn eq(&self, other: &List) -> bool {
        self.eq_at(other, 0)
    }
}

impl Eq for List {}

// SAFETY: the buffer is owned outright and reached only through `&self`
// or `&mut self`; the allocator it recorded outlives it and may be
// called from any thread, which is the contract on `Allocator`.
unsafe impl Send for List {}
// SAFETY: as above.
unsafe impl Sync for List {}

impl Default for List {
    fn default() -> List {
        List::new()
    }
}

impl From<List> for Value {
    /// Safe for the same reason [`From<Map>`](super::Map) is: an empty container
    /// owns nothing.
    fn from(list: List) -> Value {
        let mut v = Value::blank(Tag::GUATIAO_LIST);
        v.payload = Payload::list(list);
        v
    }
}
