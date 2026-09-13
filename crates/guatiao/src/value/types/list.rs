// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A sequence of values: the borrowed view and the owned container.

#![allow(missing_docs)]

use std::mem::ManuallyDrop;
use std::ptr;

use crate::value::alloc::Allocator;
// The raw layer this crate keeps to itself: the free walk, the
// allocator-taking mutators and the private helpers. Imported whole
// because the impl below calls into it at almost every line.
use crate::value::alloc::Alloc;
use crate::value::mutate::*;
use crate::value::raw::{dangling, release_buffer};

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
    pub ptr: *mut Value,
    /// Number of elements.
    pub len: usize,
    /// Capacity in elements. 0 means the buffer is not owned.
    pub cap: usize,
    /// The allocator that made this buffer. Null when `cap == 0`.
    pub alloc: *const Allocator,
}

impl Drop for List {
    fn drop(&mut self) {
        for i in 0..self.len {
            // SAFETY: the first `len` elements are initialised, and each
            // is read exactly once — `len` is zeroed below, so nothing
            // reads the buffer again.
            let elem = unsafe { self.ptr.add(i).read() };
            // Each element is a `Value`, so this is `value_free`'s
            // ITERATIVE walk. The recursion stops here at depth one, which
            // is the whole reason that walk is iterative.
            drop(elem);
        }
        self.len = 0;
        // SAFETY: the elements have been moved out.
        unsafe { release_buffer(self) }
    }
}

impl List {
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
        let mut value = value.into();
        self.as_node(|node| {
            // SAFETY: `node` is this list, and `value` is consumed here.
            unsafe { list_push(node, &mut value, alloc) }
        })
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

    /// Removes the element at `index`, keeping the order of the rest.
    pub fn remove(&mut self, index: usize) -> Option<Value> {
        // SAFETY: `self` is a well-formed list by construction.
        self.as_node(|node| unsafe { list_remove(node, index) })
    }

    /// Removes the last element, as `Vec::pop` does.
    pub fn pop(&mut self) -> Option<Value> {
        self.remove(self.len.checked_sub(1)?)
    }

    /// Frees every element, **keeping the capacity** already paid for.
    pub fn clear(&mut self) {
        // SAFETY: as above.
        let _ = self.as_node(|node| unsafe { list_clear(node) });
    }

    /// See [`Map::as_node`] for why this exists.
    fn as_node<R>(&mut self, body: impl FnOnce(&mut Value) -> R) -> R {
        let empty = List {
            ptr: dangling::<Value>(),
            len: 0,
            cap: 0,
            alloc: self.alloc,
        };
        let mut node = Value::from(std::mem::replace(self, empty));
        let out = body(&mut node);
        let node = ManuallyDrop::new(node);
        // SAFETY: as for `Map::as_node`.
        *self = unsafe { ptr::read(&*node.payload.list) };
        out
    }
}

impl Default for List {
    fn default() -> List {
        List::new()
    }
}

impl From<List> for Value {
    /// Safe for the same reason [`From<Map>`](super::Map) is: an empty container
    /// owns nothing.
    fn from(list: List) -> Value {
        let mut v = blank(Tag::GUATIAO_LIST);
        v.payload = Payload {
            list: ManuallyDrop::new(list),
        };
        v
    }
}
