// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`Wipe`]: overwriting what a value holds before it is freed.

#![forbid(unsafe_code)]

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::raw::{wipe_bytes, wipe_scalar, wipe_vec};
use super::types::{Buffer, List, Map, Text, Value};

/// Overwrites with zeros every byte a value holds, then leaves it empty.
///
/// What [`Secret`](crate::Secret) runs when it is dropped. The writes are
/// volatile and fenced, so they are not removed as stores to memory about
/// to be freed.
///
/// **It reaches only the storage the value holds now.** A `String` that
/// grew before it was wiped left its earlier blocks with the global
/// allocator, unwiped; a copy made elsewhere is that copy's. A guatiao
/// container built in [`Alloc::wiping`](crate::Alloc::wiping) has none of
/// those: every block it ever released was wiped by its allocator.
pub trait Wipe {
    /// Overwrites every byte this holds with zeros and leaves it empty
    /// (an empty text, list or map, a null value, zero).
    fn wipe(&mut self);
}

impl Wipe for Vec<u8> {
    /// The whole capacity, spare bytes included, then empty.
    fn wipe(&mut self) {
        wipe_vec(self);
    }
}

impl Wipe for String {
    /// The whole capacity, spare bytes included, then empty.
    fn wipe(&mut self) {
        let mut bytes = core::mem::take(self).into_bytes();
        wipe_vec(&mut bytes);
    }
}

impl Wipe for Box<str> {
    fn wipe(&mut self) {
        let mut bytes = core::mem::take(self).into_string().into_bytes();
        wipe_vec(&mut bytes);
    }
}

impl Wipe for Box<[u8]> {
    fn wipe(&mut self) {
        let mut bytes = core::mem::take(self).into_vec();
        wipe_vec(&mut bytes);
    }
}

macro_rules! wipe_scalars {
    ($($t:ty),*) => {$(
        impl Wipe for $t {
            /// Set to its default with a store the compiler keeps.
            fn wipe(&mut self) {
                wipe_scalar(self);
            }
        }
    )*};
}

wipe_scalars!(
    bool, char, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
);

impl<T: Wipe> Wipe for Option<T> {
    /// What it holds, then `None`.
    fn wipe(&mut self) {
        if let Some(inner) = self {
            inner.wipe();
        }
        *self = None;
    }
}

impl<T: Wipe> Wipe for Box<T> {
    fn wipe(&mut self) {
        (**self).wipe();
    }
}

impl Wipe for Value {
    /// Every text, number, bytes and map key in the tree, at any depth,
    /// whatever allocator each lives in; then the tree is freed and this
    /// is null. Bytes a node does not own (a literal) are not written.
    fn wipe(&mut self) {
        self.wipe_bytes_in_place();
        *self = Value::null();
    }
}

impl Wipe for Text {
    fn wipe(&mut self) {
        wipe_bytes(self);
        self.release();
    }
}

impl Wipe for Buffer {
    fn wipe(&mut self) {
        wipe_bytes(self);
        self.release();
    }
}

impl Wipe for List {
    /// Every element, as [`Value`]'s; then empty.
    fn wipe(&mut self) {
        for item in self.iter_mut() {
            item.wipe_bytes_in_place();
        }
        self.clear();
    }
}

impl Wipe for Map {
    /// Every key and value, as [`Value`]'s; then empty.
    fn wipe(&mut self) {
        for entry in self.entries_mut() {
            wipe_bytes(&mut entry.key);
            entry.value.wipe_bytes_in_place();
        }
        self.clear();
    }
}
