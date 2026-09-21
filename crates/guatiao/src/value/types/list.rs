// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A sequence of values: the borrowed view and the owned container.

#![allow(missing_docs)]

use std::ptr;

use crate::value::alloc::{Alloc, Allocator};
use crate::value::error::ValueError;
use crate::value::raw::{dangling, release_buffer, reserve};

use super::{Payload, Tag, Value, or_abort};

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

    /// An empty list with room for `capacity` elements, through `alloc`.
    pub(crate) fn with_capacity_in(alloc: Alloc, capacity: usize) -> Result<List, ValueError> {
        let mut list = List::new_in(alloc);
        if capacity > 0 {
            // SAFETY: the container is consistent and empty.
            unsafe { reserve(&mut list, capacity, Some(alloc))? };
        }
        Ok(list)
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
    pub(crate) fn push_node(&mut self, value: Value, alloc: Alloc) -> Result<(), ValueError> {
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

    /// A deep copy through `alloc`, at any depth.
    pub fn clone_in(&self, alloc: Alloc) -> Result<List, ValueError> {
        crate::value::walk::copy_list(self, alloc)
    }

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
}

/// The elements, as `Vec` gives them: `len`, `get`, `iter`, indexing and
/// every other slice method are the slice's own.
impl std::ops::Deref for List {
    type Target = [Value];

    fn deref(&self) -> &[Value] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` elements are initialised, and the borrow
        // of `self` keeps them alive and unaliased.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

/// The elements, mutably. A slice cannot change its length, so what this
/// list owns and how it frees it are untouched: an element moved out of a
/// slot is moved out whole, as `mem::replace` or `swap` does.
impl std::ops::DerefMut for List {
    fn deref_mut(&mut self) -> &mut [Value] {
        if self.len == 0 {
            return &mut [];
        }
        // SAFETY: as for `deref`, and `&mut self` makes the borrow unique.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl AsRef<[Value]> for List {
    fn as_ref(&self) -> &[Value] {
        self
    }
}

impl AsMut<[Value]> for List {
    fn as_mut(&mut self) -> &mut [Value] {
        self
    }
}

/// The elements, moved out in order. What is not taken is freed with the
/// iterator.
#[derive(Debug)]
pub struct IntoIter {
    /// The rest, in reverse, so each `next` is a `pop`.
    rest: List,
}

impl Iterator for IntoIter {
    type Item = Value;

    fn next(&mut self) -> Option<Value> {
        self.rest.pop()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.rest.len(), Some(self.rest.len()))
    }
}

impl ExactSizeIterator for IntoIter {}

impl IntoIterator for List {
    type Item = Value;
    type IntoIter = IntoIter;

    fn into_iter(mut self) -> IntoIter {
        self.reverse();
        IntoIter { rest: self }
    }
}

impl<'a> IntoIterator for &'a mut List {
    type Item = &'a mut Value;
    type IntoIter = std::slice::IterMut<'a, Value>;

    fn into_iter(self) -> std::slice::IterMut<'a, Value> {
        self.iter_mut()
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
    /// Element by element, each by [`Value`]'s own comparison.
    fn eq(&self, other: &List) -> bool {
        crate::value::walk::equal(crate::value::walk::Pair::Lists(self, other))
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

/// Collects anything a value is made from, as `Vec` does. Grows through
/// Rust's allocator and aborts if it refuses, the short form's policy.
impl<T: Into<Value>> FromIterator<T> for List {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> List {
        let mut list = List::new();
        list.extend(iter);
        list
    }
}

/// Appends each item through the allocator this list recorded. Panics if
/// it refuses, which for a list over storage another language owns is
/// every time: that list has no allocator to grow through.
impl<T: Into<Value>> Extend<T> for List {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            or_abort(self.push(item));
        }
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

#[cfg(test)]
mod collect_tests {
    use super::*;
    use crate::value::convert::TryAsRef;
    use crate::value::types::Map;

    #[test]
    fn a_list_collects_anything_a_value_is_made_from() {
        let list: List = ["a", "b"].into_iter().collect();
        let texts: Vec<&str> = list
            .iter()
            .filter_map(TryAsRef::<str>::try_as_ref)
            .collect();
        assert_eq!(texts, ["a", "b"]);

        let mut more = List::new();
        more.extend([1, 2, 3]);
        assert_eq!(more.len(), 3);
    }

    #[test]
    fn a_map_collects_pairs_in_order_and_a_repeat_replaces_in_place() {
        let map: Map = [("b", 1), ("a", 2), ("b", 3)].into_iter().collect();
        let keys: Vec<&str> = map.keys().collect();
        assert_eq!(
            keys,
            ["b", "a"],
            "insertion order, the repeat kept its slot"
        );
        assert_eq!(i64::try_from(map.get("b").unwrap()).unwrap(), 3);
    }
}
